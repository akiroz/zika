use base64::{engine::general_purpose, Engine as _};
use futures::stream::StreamExt;
use rand::distributions::Standard;
use rand::{thread_rng, Rng};
use std::io::Read;
use std::sync::Arc;
use tokio::{sync::mpsc, task};
use tun::{IntoAddress as _, TunPacket};

use crate::config;
use crate::remote;

pub struct Client {
    tunnels: Vec<Arc<Tunnel>>,
    receiver: mpsc::Receiver<TunPacket>,
    remote: Arc<remote::Remote>,
}

struct Tunnel {
    id: Vec<u8>,
    topic: String,
    bind_addr: std::net::Ipv4Addr,
}

impl Client {
    pub async fn new(config: &config::Config) -> Self {
        let mqtt_options = config.broker_mqtt_options();

        let (remote, _) = remote::Remote::new(&mqtt_options, Vec::new());
        let arc_remote = Arc::new(remote);

        let client_config = config
            .client
            .as_ref()
            .expect("Client config to be non-null");

        let (sender, receiver) = mpsc::channel(128);
        let mut tunnels = Vec::with_capacity(client_config.tunnels.len());
        let mut rng = thread_rng();

        for client_tunnel_config in &client_config.tunnels {
            let mut tun_config = tun::Configuration::default();
            tun_config.address(client_tunnel_config.bind_addr.clone());
            if let Some(driver_tun_config) = &config.driver.tun {
                tun_config.netmask(driver_tun_config.netmask.clone());
            }
            tun_config.up();

            #[cfg(target_os = "linux")]
            tun_config.platform(|tun_config| {
                tun_config.packet_information(true);
            });

            let dev = tun::create_as_async(&tun_config).unwrap();
            let mut stream = dev.into_framed();

            let random_id: Vec<u8> = (&mut rng)
                .sample_iter(Standard)
                .take(client_tunnel_config.id_length)
                .collect();

            let base64_id = general_purpose::STANDARD.encode(&random_id);
            let topic_base = &client_tunnel_config.topic;
            let topic = format!("{topic_base}/{base64_id}");
            let subscribe_result = arc_remote.subscribe(topic).await;
            if let Err(err) = subscribe_result {
                log::error!("Subscribe failed: {:?}", err);
            }

            let tunnel = Tunnel {
                id: random_id,
                topic: client_tunnel_config.topic.clone(),
                bind_addr: client_tunnel_config.bind_addr.into_address().unwrap(),
            };
            let arc_tunnel = Arc::new(tunnel);

            let loop_sender = sender.clone();
            let loop_remote = arc_remote.clone();
            let loop_tunnel = arc_tunnel.clone();

            task::spawn(async move {
                while let Some(packet) = stream.next().await {
                    match packet {
                        Ok(pkt) => {
                            Self::handle_packet(&loop_remote, &loop_tunnel, &pkt).await;
                            let _ = loop_sender.clone().send(pkt).await;
                        }
                        Err(err) => panic!("Error: {:?}", err),
                    }
                }
            });

            tunnels.push(arc_tunnel.clone());
        }

        Client {
            tunnels: tunnels,
            receiver: receiver,
            remote: arc_remote.clone(),
        }
    }

    async fn handle_packet(remote: &remote::Remote, tunnel: &Tunnel, packet: &TunPacket) {
        let dest = etherparse::IpHeader::from_slice(&packet.get_bytes())
            .ok()
            .and_then(|header| match header {
                (etherparse::IpHeader::Version4(ipv4_header, _), _, _) => {
                    let tuple: (u8, u8, u8, u8) = ipv4_header.destination.into();
                    tuple.into_address().ok()
                }
                _ => None,
            });

        if dest == Some(tunnel.bind_addr) {
            let payload = [&tunnel.id[..], &packet.get_bytes().to_vec()[..]].concat();
            let result = remote.publish(&tunnel.topic, payload).await;
            log::debug!("published {:?}", result);
        }
    }

    pub async fn run(&mut self) {
        loop {
            let x = self.receiver.recv().await;
            log::debug!("recv = {:?}", x);
        }
    }
}
