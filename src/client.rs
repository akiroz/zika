use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::Bytes;
use base64::{engine::general_purpose, Engine as _};
use futures::{SinkExt, stream::{SplitSink, StreamExt}};
use rand::{thread_rng, Rng, distributions::Standard};

use rumqttc;
use etherparse::Ipv4Header;
use ipnetwork::Ipv4Network;
use tokio::{task, sync::{broadcast, Mutex}};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, TunPacket, TunPacketCodec};

use crate::config;
use crate::remote;
use crate::ip_iter::SizedIpv4NetworkIterator;

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;

pub struct Client {
    pub local_addr: Ipv4Addr,
    tunnels: Arc<Vec<Tunnel>>,
    pub remote: Arc<Mutex<remote::Remote>>, // Allow external mqtt ops
    pub remote_passthru_recv: Arc<Mutex<broadcast::Receiver<(String, Bytes)>>>,
}

struct Tunnel {
    id: Vec<u8>,
    topic: String,
    topic_base: String,
    bind_addr: Ipv4Addr,
}

impl Client {
    pub async fn from_config(config: config::Config) -> Arc<Self> {
        let mqtt_options = config.mqtt.to_mqtt_options().expect("valid mqtt config");
        let client_config = config.client.expect("non-null client config");
        Self::new(mqtt_options, client_config).await
    }

    pub async fn new(
        mqtt_options: Vec<rumqttc::v5::MqttOptions>,
        client_config: config::ClientConfig
    ) -> Arc<Self> {
        let ip_network: Ipv4Network = client_config.bind_cidr.parse().expect("CIDR notation");
        let local_addr = SizedIpv4NetworkIterator::new(ip_network).next().expect("subnet size > 1");

        log::info!("tun {:?}/{}", local_addr, ip_network.prefix());
        let mut tun_config = tun::Configuration::default();
        tun_config.address(local_addr);
        tun_config.destination(local_addr);
        tun_config.netmask(ip_network.mask());

        #[cfg(target_os = "linux")]
        tun_config.platform(|tun_config| {
            tun_config.packet_information(true);
        });

        tun_config.up();
        let tun_dev = tun::create_as_async(&tun_config).expect("tunnel");
        let (mut tun_sink, mut tun_stream) = tun_dev.into_framed().split();

        let (remote, mut remote_recv) = remote::Remote::new(&mqtt_options, vec![]);
        let mut tunnels = Vec::with_capacity(client_config.tunnels.len());
        let mut rng = thread_rng();
        for client_tunnel_config in &client_config.tunnels {
            let id_len = client_tunnel_config.id_length;
            let random_id: Vec<u8> = (0..id_len).map(|_| rng.sample(Standard)).collect();
            let base64_id = general_purpose::URL_SAFE_NO_PAD.encode(&random_id);
            let topic_base = &client_tunnel_config.topic;
            let topic = format!("{topic_base}/{base64_id}");
            let bind_addr = client_tunnel_config.bind_addr;

            if bind_addr == local_addr {
                panic!("tunnel bind_addr == local_addr, first address in subnet is reserved");
            }
            if !ip_network.contains(bind_addr) {
                panic!("tunnel bind_addr outside subnet");
            }
            log::info!("bind {:?} -> {:?}", &bind_addr, &topic);

            let subscribe_result = remote.subscribe(topic.clone()).await;
            if let Err(err) = subscribe_result {
                log::error!("subscribe failed: {:?}", err);
            }

            tunnels.push(Tunnel {
                id: random_id,
                topic,
                topic_base: topic_base.to_string(),
                bind_addr,
            });
        }

        let (remote_passthru_send, remote_passthru_recv) = broadcast::channel(1);
        let client = Arc::new(Self {
            local_addr,
            tunnels: Arc::new(tunnels),
            remote: Arc::new(Mutex::new(remote)),
            remote_passthru_recv: Arc::new(Mutex::new(remote_passthru_recv)),
        });

        let loop_client = client.clone();
        let loop_remote = client.remote.clone();
        task::spawn(async move {
            while let Some(packet) = tun_stream.next().await {
                match packet {
                    Ok(pkt) => {
                        let mut remote = loop_remote.lock().await;
                        loop_client.handle_packet(&mut remote, &pkt).await;
                    }
                    Err(err) => panic!("Error: {:?}", err),
                }
            }
        });

        let loop2_client = client.clone();
        task::spawn(async move {
            loop {
                if let Some((topic, msg)) = remote_recv.recv().await {
                    let handle_result = loop2_client.handle_remote_message(&mut tun_sink, topic.as_str(), &msg).await;
                    match handle_result {
                        Err(err) => log::error!("handle_remote_message error {:?}", err),
                        Ok(handled) => {
                            if !handled {
                                if let Err(err) = remote_passthru_send.send((topic, msg)) {
                                    log::warn!("remote_passthru_send error {:?}", err);
                                }
                            }
                        }
                    }
                } else {
                    break;
                }
            }
        });

        client
    }

    // tun -> mqtt
    async fn handle_packet(&self, remote: &mut remote::Remote, packet: &TunPacket) {
        let dest = etherparse::IpHeader::from_slice(&packet.get_bytes())
            .ok()
            .and_then(|header| match header {
                (etherparse::IpHeader::Version4(ipv4_header, _), _, _) => {
                    Some(Ipv4Addr::from(ipv4_header.destination))
                }
                _ => None,
            });
        for tunnel in self.tunnels.iter() {
            if dest == Some(tunnel.bind_addr) {
                let payload = [&tunnel.id[..], &packet.get_bytes().to_vec()[..]].concat();
                let result = remote.publish(&tunnel.topic_base, payload).await;
                if let Err(result) = result {
                    log::error!("publish failed {:?}", result);
                }
                return;
            }
        }
        log::warn!("drop packet: unknown addr {:?}", &dest);
    }

    // mqtt -> tun
    async fn handle_remote_message(&self, tun_sink: &mut TunSink, topic: &str, msg: &[u8]) -> Result<bool, etherparse::WriteError> {
        if let Some(tunnel) = self.tunnels.iter().find(|&t| t.topic == topic) {
            match Ipv4Header::from_slice(&msg) {
                Err(error) => {
                    log::debug!("packet parse failed {:?}", error);
                    Ok(false)
                }
                Ok((mut ipv4_header, rest)) => {
                    ipv4_header.source = tunnel.bind_addr.octets();
                    ipv4_header.destination = self.local_addr.octets();
                    let mut cursor = Cursor::new(Vec::new());
                    ipv4_header.write(&mut cursor)?;
                    cursor.write_all(rest)?;
                    tun_sink.send(TunPacket::new(cursor.into_inner())).await?;
                    Ok(true)
                }
            }
        } else {
            Ok(false)
        }
    }
}
