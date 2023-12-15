use crate::config;
use crate::ip_iter::SizedIpv4NetworkIterator;
use crate::lookup_pool::LookupPool;
use crate::remote;
use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;
use etherparse::Ipv4Header;
use futures::stream::{SplitSink, StreamExt};
use futures::SinkExt;
use ipnetwork::Ipv4Network;
use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{sync::mpsc, task};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, IntoAddress as _, TunPacket, TunPacketCodec};

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;
type IpPool = LookupPool<String, Ipv4Addr, SizedIpv4NetworkIterator>;

pub struct Server {
    remote_receiver: mpsc::Receiver<(String, Bytes)>,
    topic: String,
    local_addr: Ipv4Addr,
    id_length: usize,
    ip_pool: Arc<Mutex<IpPool>>,
    sink: TunSink,
}

impl Server {
    pub fn new(config: config::Config) -> Self {
        let mqtt_options = config.broker_mqtt_options();
        let server_config = config.server.expect("Server config to be non-null");

        let (remote, remote_receiver) =
            remote::Remote::new(&mqtt_options, vec![server_config.topic.clone()]);
        let netmask = config.driver.tun.netmask;
        let ip_network = Ipv4Network::with_netmask(server_config.bind_addr, netmask)
            .expect("A proper ip network");
        let bind_addr = ip_network.ip();

        log::info!("Binding to {:?} netmask {:?}", bind_addr, netmask);

        let ip_iter = SizedIpv4NetworkIterator::new(ip_network);

        let mut tun_config = tun::Configuration::default();
        tun_config.address(bind_addr);
        tun_config.destination(bind_addr);
        tun_config.netmask(netmask);

        #[cfg(target_os = "linux")]
        tun_config.platform(|tun_config| {
            tun_config.packet_information(true);
        });

        tun_config.up();

        let dev = tun::create_as_async(&tun_config).expect("Tunnel");
        let (sink, mut stream) = dev.into_framed().split();

        let ip_pool_arc = Arc::new(Mutex::new(LookupPool::new(ip_iter)));
        let loop_ip_pool_arc = ip_pool_arc.clone();

        task::spawn(async move {
            while let Some(packet) = stream.next().await {
                match packet {
                    Ok(pkt) => {
                        let result =
                            Self::handle_packet(&remote, loop_ip_pool_arc.clone(), &pkt).await;
                        if let Err(err) = result {
                            log::error!("Error in handle_packet {:?}", err);
                        }
                    }
                    Err(err) => panic!("Cannot get packet from tun, error: {:?}", err),
                }
            }
        });

        Self {
            remote_receiver,
            topic: server_config.topic,
            local_addr: config.driver.local_addr,
            id_length: server_config.id_length,
            ip_pool: ip_pool_arc,
            sink,
        }
    }

    // tun -> mqtt
    async fn handle_packet(
        remote: &remote::Remote,
        ip_pool: Arc<Mutex<IpPool>>,
        packet: &TunPacket,
    ) -> Result<(), rumqttc::v5::ClientError> {
        let dest = Ipv4Header::from_slice(&packet.get_bytes())
            .ok()
            .and_then(|(ipv4_header, _)| {
                let tuple: (u8, u8, u8, u8) = ipv4_header.destination.into();
                tuple.into_address().ok()
            });
        if let Some(d) = dest {
            let ip_pool = ip_pool.lock().await;
            if let Some(topic) = ip_pool.get_reverse(&d.into()) {
                remote.publish(&topic, packet.get_bytes().to_vec()).await?;
            } else {
                log::debug!("Cannot find topic for {:?}", &d);
            }
        }
        Ok(())
    }

    // mqtt -> tun
    async fn handle_remote_message(
        &mut self,
        id: &[u8],
        message: &[u8],
    ) -> Result<(), etherparse::WriteError> {
        let base64_id = general_purpose::URL_SAFE_NO_PAD.encode(id);
        let topic_base = &self.topic;
        let topic = format!("{topic_base}/{base64_id}");
        let ip: Ipv4Addr = {
            let mut ip_pool = self.ip_pool.lock().await;
            let ip = ip_pool.get_forward(&topic).into();
            ip
        };
        let packet_with_updated_header = match Ipv4Header::from_slice(message) {
            Err(error) => {
                log::debug!("Cannot parse ip packet {:?}", error);
                message.to_vec()
            }
            Ok((mut ipv4_header, rest)) => {
                ipv4_header.source = ip.octets();
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
        Ok(())
    }

    pub async fn run(&mut self) {
        loop {
            if let Some((_topic, id_message)) = self.remote_receiver.recv().await {
                let (id, message) = id_message.split_at(self.id_length);
                if let Err(err) = self.handle_remote_message(id, message).await {
                    log::error!("Error in handle_remote_message {:?}", err);
                }
            } else {
                break;
            }
        }
    }
}
