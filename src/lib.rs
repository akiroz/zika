pub mod config;
mod lookup_pool;
mod remote;

use etherparse::Ipv4Header;
use futures::stream::StreamExt;
use packet::ip::Packet;
use std::io::Read;

pub struct Client {
    frame: tokio_util::codec::Framed<tun::AsyncDevice, tun::TunPacketCodec>,
}

impl Client {
    pub fn new(
        client_tunnel_config: &config::ClientTunnel,
        driver_config: &config::DriverConfig,
    ) -> Self {
        let mut config = tun::Configuration::default();
        config.address(client_tunnel_config.bind_addr.clone());
        if let Some(tun_config) = &driver_config.tun {
            config.netmask(tun_config.netmask.clone());
        }
        config.up();

        #[cfg(target_os = "linux")]
        config.platform(|config| {
            config.packet_information(true);
        });

        let dev = tun::create_as_async(&config).unwrap();
        let stream = dev.into_framed();
        Self { frame: stream }
    }

    pub async fn run(&mut self) {
        while let Some(packet) = self.frame.next().await {
            match packet {
                Ok(pkt) => log::debug!("pkt: {:#?}", Packet::unchecked(pkt.get_bytes())),
                Err(err) => panic!("Error: {:?}", err),
            }
        }
    }
}
pub mod server;
