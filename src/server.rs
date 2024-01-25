use std::error::Error as StdError;
use std::net::Ipv4Addr;
use std::sync::Arc;

use base64::{engine::general_purpose, Engine as _};
use futures::{SinkExt, stream::{SplitSink, StreamExt}};

use etherparse::Ipv4Header;
use ipnetwork::Ipv4Network;
use tokio::{task, sync::Mutex};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, TunPacket, TunPacketCodec};

use crate::nat;
use crate::config;
use crate::remote;
use crate::ip_iter::SizedIpv4NetworkIterator;
use crate::lookup_pool::LookupPool;

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;
type IpPool = LookupPool<String, Ipv4Addr, SizedIpv4NetworkIterator>;

pub struct Server {
    pub id_length: usize,
    pub topic: String,
    pub local_addr: Ipv4Addr,
    ip_pool: Arc<Mutex<IpPool>>,
}

impl Server {
    pub fn from_config(config: config::Config) -> Arc<Self> {
        let mqtt_options = config.mqtt.to_mqtt_options().expect("valid mqtt config");
        let server_config = config.server.expect("non-null server config");
        Self::new(mqtt_options, server_config)
    }

    pub fn new(
        mqtt_options: Vec<rumqttc::v5::MqttOptions>,
        server_config: config::ServerConfig
    ) -> Arc<Self> {
        let ip_network: Ipv4Network = server_config.bind_cidr.parse().expect("CIDR notation");
        let mut ip_iter = SizedIpv4NetworkIterator::new(ip_network);
        let local_addr = ip_iter.next().expect("subnet size > 1");

        let (mut remote, mut remote_recv) = remote::Remote::new(&mqtt_options, vec![server_config.topic.clone()]);

        log::info!("bind {:?}/{}", local_addr, ip_network.prefix());

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
        let (mut tun_sink, mut tun_stream) = dev.into_framed().split();

        let server = Arc::new(Self {
            id_length: server_config.id_length,
            topic: server_config.topic,
            local_addr,
            ip_pool: Arc::new(Mutex::new(LookupPool::new(ip_iter))),
        });

        let loop_ip_pool = server.ip_pool.clone();
        task::spawn(async move {
            while let Some(packet) = tun_stream.next().await {
                match packet {
                    Ok(pkt) => {
                        let mut ip_pool = loop_ip_pool.lock().await;
                        let result = Self::handle_packet(&mut remote, &mut ip_pool, &pkt).await;
                        if let Err(err) = result {
                            log::error!("handle_packet error {:?}", err);
                        }
                    }
                    Err(err) => panic!("Cannot get packet from tun, error: {:?}", err),
                }
            }
        });

        let loop_server = server.clone();
        task::spawn(async move {
            loop {
                if let Some((_topic, payload)) = remote_recv.recv().await {
                    let (id, msg) = payload.split_at(server_config.id_length);
                    let handle_result = loop_server.handle_remote_message(&mut tun_sink, id, msg).await;
                    if let Err(err) = handle_result {
                        log::error!("handle_remote_message error {:?}", err);
                    }
                } else {
                    break;
                }
            }
        });

        server
    }

    // tun -> mqtt
    async fn handle_packet(remote: &mut remote::Remote, ip_pool: &mut IpPool, pkt: &TunPacket) -> Result<(), rumqttc::v5::ClientError> {
        let dest = Ipv4Header::from_slice(&pkt.get_bytes())
            .ok()
            .map(|(ipv4_header, _)| Ipv4Addr::from(ipv4_header.destination));
        if let Some(d) = dest {
            if let Some(topic) = ip_pool.get_reverse(&d.into()) {
                remote.publish(&topic, pkt.get_bytes().to_vec()).await?;
            } else {
                log::debug!("drop packet: no tunnel for {:?}", &d);
            }
        }
        Ok(())
    }

    // mqtt -> tun
    async fn handle_remote_message(&self, tun_sink: &mut TunSink, id: &[u8], msg: &[u8]) -> Result<(), Box<dyn StdError>> {
        let base64_id = general_purpose::URL_SAFE_NO_PAD.encode(id);
        let topic_base = &self.topic;
        let topic = format!("{topic_base}/{base64_id}");
        let ip: Ipv4Addr = {
            let mut ip_pool = self.ip_pool.lock().await;
            let ip = ip_pool.get_forward(&topic).into();
            ip
        };
        let pkt = nat::do_nat(msg, ip, self.local_addr)?;
        tun_sink.send(TunPacket::new(pkt)).await?;
        Ok(())
    }
}
