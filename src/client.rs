use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;
use etherparse::Ipv4Header;
use futures::stream::{SplitSink, StreamExt};
use futures::SinkExt;
use ipnetwork::Ipv4Network;
use rand::distributions::Standard;
use rand::{thread_rng, Rng};
use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::{sync::mpsc, task};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, TunPacket, TunPacketCodec};

use crate::config;
use crate::ip_iter::SizedIpv4NetworkIterator;
use crate::remote;

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;

pub struct Client {
    tunnels: Arc<Vec<Tunnel>>,
    remote_receiver: mpsc::Receiver<(String, Bytes)>,
    local_addr: Ipv4Addr,
    sink: TunSink,
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

        let dev = tun::create_as_async(&tun_config).expect("tunnel");
        let (sink, mut stream) = dev.into_framed().split();

        let mqtt_options = config.broker_mqtt_options();

        let (mut remote, remote_receiver) = remote::Remote::new(&mqtt_options, Vec::new());

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

        let arc_tunnels = Arc::new(tunnels);
        let loop_tunnels = arc_tunnels.clone();

        task::spawn(async move {
            while let Some(packet) = stream.next().await {
                match packet {
                    Ok(pkt) => {
                        Self::handle_packet(&mut remote, &loop_tunnels, &pkt).await;
                    }
                    Err(err) => panic!("Error: {:?}", err),
                }
            }
        });

        Client {
            tunnels: arc_tunnels.clone(),
            remote_receiver,
            local_addr,
            sink,
        }
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
    ) -> Result<(), etherparse::WriteError> {
        let parsed = Ipv4Header::from_slice(&message);
        for tunnel in self.tunnels.as_ref() {
            if tunnel.topic != topic {
                continue;
            }
            let packet_with_updated_header = match parsed {
                Err(error) => {
                    log::debug!("packet parse failed {:?}", error);
                    message.to_vec()
                }
                Ok((mut ipv4_header, rest)) => {
                    ipv4_header.source = tunnel.bind_addr.octets();
                    ipv4_header.destination = self.local_addr.octets();
                    let mut cursor = Cursor::new(Vec::new());
                    ipv4_header.write(&mut cursor)?;
                    cursor.write_all(rest)?;
                    cursor.into_inner()
                }
            };
            self.sink
                .send(TunPacket::new(packet_with_updated_header))
                .await?;
            break;
        }
        Ok(())
    }

    pub async fn run(&mut self) {
        loop {
            if let Some((topic, message)) = self.remote_receiver.recv().await {
                if let Err(err) = self.handle_remote_message(topic, message).await {
                    log::error!("handle_remote_message error {:?}", err);
                }
            } else {
                break;
            }
        }
    }
}
