use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::Bytes;
use base64::{engine::general_purpose, Engine as _};
use futures::{SinkExt, stream::{SplitSink, StreamExt}};
use rand::{thread_rng, Rng, distributions::Standard};

use etherparse::Ipv4Header;
use ipnetwork::Ipv4Network;
use tokio::{task, sync::{mpsc, broadcast, Mutex}};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, TunPacket, TunPacketCodec};

use crate::config;
use crate::ip_iter::SizedIpv4NetworkIterator;
use crate::remote;

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;

pub struct Client {
    local_addr: Ipv4Addr,
    tunnels: Arc<Vec<Tunnel>>,
    pub remote: Arc<Mutex<remote::Remote>>, // Allow external mqtt ops
    remote_recv: mpsc::Receiver<(String, Bytes)>,
    remote_passthru_send: broadcast::Sender<(String, Bytes)>,
    pub remote_passthru_recv: broadcast::Receiver<(String, Bytes)>,
    tun_sink: TunSink,
}

struct Tunnel {
    id: Vec<u8>,
    topic: String,
    topic_base: String,
    bind_addr: std::net::Ipv4Addr,
}

impl Client {
    pub async fn new(config: &config::Config) -> Self {
        let client_config = config
            .client
            .as_ref()
            .expect("non-null config");

        let ip_network: Ipv4Network = client_config
            .bind_cidr
            .parse()
            .expect("CIDR notation");
        let local_addr = SizedIpv4NetworkIterator::new(ip_network)
            .next()
            .expect("subnet size > 1");

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
        let (tun_sink, mut tun_stream) = tun_dev.into_framed().split();

        let mqtt_options = config.broker_mqtt_options();

        let (remote, remote_recv) = remote::Remote::new(&mqtt_options, Vec::new());

        let mut tunnels = Vec::with_capacity(client_config.tunnels.len());
        let mut rng = thread_rng();

        for client_tunnel_config in &client_config.tunnels {
            let random_id: Vec<u8> = (&mut rng)
                .sample_iter(Standard)
                .take(client_tunnel_config.id_length)
                .collect();
            let base64_id = general_purpose::URL_SAFE_NO_PAD.encode(&random_id);
            let topic_base = &client_tunnel_config.topic;
            let topic = format!("{topic_base}/{base64_id}");
            let bind_addr = client_tunnel_config.bind_addr;

            if !ip_network.contains(bind_addr) {
                log::warn!("skipping {:?} (outside subnet)", bind_addr);
                continue;
            }
            log::info!("bind {:?} -> {:?}", &bind_addr, &topic);

            let subscribe_result = remote.subscribe(topic.clone()).await;
            if let Err(err) = subscribe_result {
                log::error!("subscribe failed: {:?}", err);
            }

            let tunnel = Tunnel {
                id: random_id,
                topic,
                topic_base: topic_base.to_string(),
                bind_addr,
            };

            tunnels.push(tunnel);
        }

        let (remote_passthru_send, remote_passthru_recv) = broadcast::channel(1);
        let client = Client {
            local_addr,
            tunnels: Arc::new(tunnels),
            remote: Arc::new(Mutex::new(remote)),
            remote_recv,
            remote_passthru_send,
            remote_passthru_recv,
            tun_sink,
        };

        let loop_remote = client.remote.clone();
        let loop_tunnels = client.tunnels.clone();
        task::spawn(async move {
            while let Some(packet) = tun_stream.next().await {
                match packet {
                    Ok(pkt) => {
                        let mut remote = loop_remote.lock().await;
                        Self::handle_packet(&mut remote, &loop_tunnels, &pkt).await;
                    }
                    Err(err) => panic!("Error: {:?}", err),
                }
            }
        });

        client
    }

    // tun -> mqtt
    async fn handle_packet(remote: &mut remote::Remote, tunnels: &Vec<Tunnel>, packet: &TunPacket) {
        let dest = etherparse::IpHeader::from_slice(&packet.get_bytes())
            .ok()
            .and_then(|header| match header {
                (etherparse::IpHeader::Version4(ipv4_header, _), _, _) => {
                    Some(Ipv4Addr::from(ipv4_header.destination))
                }
                _ => None,
            });
        for tunnel in tunnels {
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
    async fn handle_remote_message(
        &mut self,
        topic: String,
        message: Bytes,
    ) -> Result<bool, etherparse::WriteError> {
        if let Some(tunnel) = self.tunnels.as_ref().iter().find(|&t| t.topic == topic) {
            match Ipv4Header::from_slice(&message) {
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
                    self.tun_sink.send(TunPacket::new(cursor.into_inner())).await?;
                    Ok(true)
                }
            }
        } else {
            Ok(false)
        }
    }

    pub async fn run(&mut self) {
        loop {
            if let Some((topic, message)) = self.remote_recv.recv().await {
                match self.handle_remote_message(topic.clone(), message.clone()).await {
                    Err(err) => {
                        log::error!("handle_remote_message error {:?}", err);
                    }
                    Ok(handled) => {
                        if !handled {
                            if let Err(err) = self.remote_passthru_send.send((topic, message)) {
                                log::warn!("remote_passthru_send error {:?}", err);
                            }
                        }
                    }
                }
            } else {
                break;
            }
        }
    }
}
