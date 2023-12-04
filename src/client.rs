use etherparse::Ipv4Header;
use futures::stream::StreamExt;
use packet::ip::Packet;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::io::Read;
use tokio::{sync::mpsc, task};
use tun::TunPacket;

use crate::config;

pub struct Client {
    tunnels: Vec<Tunnel>,
    receiver: mpsc::Receiver<TunPacket>,
}

struct Tunnel {
    id: String,
}

impl Client {
    pub fn new(config: &config::Config) -> Self {
        let mqtt_options = config.broker_mqtt_options();
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

            let random_id: String = (&mut rng)
                .sample_iter(Alphanumeric)
                .take(client_tunnel_config.id_length)
                .map(char::from)
                .collect();

            let loop_sender = sender.clone();

            task::spawn(async move {
                while let Some(packet) = stream.next().await {
                    match packet {
                        Ok(pkt) => {
                            log::debug!("pkt: {:#?}", Packet::unchecked(pkt.get_bytes()));
                            loop_sender.clone().send(pkt).await;
                        }
                        Err(err) => panic!("Error: {:?}", err),
                    }
                }
            });

            tunnels.push(Tunnel { id: random_id });
        }

        Client {
            tunnels: tunnels,
            receiver: receiver,
        }
    }

    pub async fn run(&mut self) {
        loop {
            let x = self.receiver.recv().await;
            log::debug!("recv = {:?}", x);
        }
    }
}
