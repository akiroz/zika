use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;
use etherparse::Ipv4Header;
use futures::stream::SplitSink;
use futures::stream::StreamExt;
use futures::SinkExt;
use ipnetwork::Ipv4Network;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::{sync::mpsc, task};
use tokio_util::codec::Framed;
use tun::AsyncDevice;
use tun::TunPacketCodec;
use tun::{IntoAddress as _, TunPacket};

use crate::config;
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
        log::info!(
            "Creating tun at {:?} network {:?}",
            config.driver.local_addr,
            config.driver.tun.netmask
        );
        let mut tun_config = tun::Configuration::default();
        tun_config.address(config.driver.local_addr);
        tun_config.destination(config.driver.local_addr);
        tun_config.netmask(config.driver.tun.netmask);

        #[cfg(target_os = "linux")]
        tun_config.platform(|tun_config| {
            tun_config.packet_information(true);
        });

        tun_config.up();

        let dev = tun::create_as_async(&tun_config).expect("Tunnel");
        let (sink, mut stream) = dev.into_framed().split();

        let ip_network =
            Ipv4Network::with_netmask(config.driver.local_addr, config.driver.tun.netmask)
                .expect("A proper ip network");
        let mqtt_options = config.broker_mqtt_options();

        let (remote, remote_receiver) = remote::Remote::new(&mqtt_options, Vec::new());

        let client_config = config
            .client
            .as_ref()
            .expect("Client config to be non-null");

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
                log::error!("{:?} is outside of the subnet, skipping", bind_addr);
                continue;
            }
            log::info!("Binding {:?} to {:?}", &topic, &bind_addr);

            let subscribe_result = remote.subscribe(topic.clone()).await;
            if let Err(err) = subscribe_result {
                log::error!("Subscribe failed: {:?}", err);
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
                        Self::handle_packet(&remote, &loop_tunnels, &pkt).await;
                    }
                    Err(err) => panic!("Error: {:?}", err),
                }
            }
        });

        Client {
            tunnels: arc_tunnels.clone(),
            remote_receiver,
            local_addr: config.driver.local_addr.clone(),
            sink,
        }
    }

    // tun -> mqtt
    async fn handle_packet(remote: &remote::Remote, tunnels: &Vec<Tunnel>, packet: &TunPacket) {
        let dest = etherparse::IpHeader::from_slice(&packet.get_bytes())
            .ok()
            .and_then(|header| match header {
                (etherparse::IpHeader::Version4(ipv4_header, _), _, _) => {
                    let tuple: (u8, u8, u8, u8) = ipv4_header.destination.into();
                    tuple.into_address().ok()
                }
                _ => None,
            });
        for tunnel in tunnels {
            if dest == Some(tunnel.bind_addr) {
                let payload = [&tunnel.id[..], &packet.get_bytes().to_vec()[..]].concat();
                let result = remote.publish(&tunnel.topic_base, payload).await;
                if let Err(result) = result {
                    log::error!("Cannot publish to remote {:?}", result);
                }
                return;
            }
        }
        log::warn!(
            "Received packet to {:?}, but there's no tunnel with this ip",
            &dest
        );
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
                    log::debug!("Cannot parse ip packet {:?}", error);
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
                    log::error!("Error in handle_remote_message {:?}", err);
                }
            } else {
                break;
            }
        }
    }
}
