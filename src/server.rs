use std::io::{Cursor, Write};
use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::Bytes;
use base64::{engine::general_purpose, Engine as _};
use futures::{SinkExt, stream::{SplitSink, StreamExt}};

use etherparse::Ipv4Header;
use ipnetwork::Ipv4Network;
use tokio::{task, sync::{mpsc, Mutex}};
use tokio_util::codec::Framed;
use tun::{AsyncDevice, TunPacket, TunPacketCodec};

use crate::config;
use crate::remote;
use crate::ip_iter::SizedIpv4NetworkIterator;
use crate::lookup_pool::LookupPool;

type TunSink = SplitSink<Framed<AsyncDevice, TunPacketCodec>, TunPacket>;
type IpPool = LookupPool<String, Ipv4Addr, SizedIpv4NetworkIterator>;

pub struct Server {
    id_length: usize,
    topic: String,
    local_addr: Ipv4Addr,
    ip_pool: Arc<Mutex<IpPool>>,
    remote_recv: mpsc::Receiver<(String, Bytes)>,
    tun_sink: TunSink,
}

impl Server {
    pub fn from_config(config: config::Config) -> Self {
        let mqtt_options = config.mqtt.to_mqtt_options().expect("valid mqtt config");
        let server_config = config.server.expect("non-null server config");
        Self::new(mqtt_options, server_config)
    }

    pub fn new(
        mqtt_options: Vec<rumqttc::v5::MqttOptions>,
        server_config: config::ServerConfig
    ) -> Self {
        let ip_network: Ipv4Network = server_config.bind_cidr.parse().expect("CIDR notation");
        let mut ip_iter = SizedIpv4NetworkIterator::new(ip_network);
        let local_addr = ip_iter.next().expect("subnet size > 1");

        let (mut remote, remote_recv) = remote::Remote::new(&mqtt_options, vec![server_config.topic.clone()]);

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
        let (tun_sink, mut tun_stream) = dev.into_framed().split();

        let ip_pool_arc = Arc::new(Mutex::new(LookupPool::new(ip_iter)));
        let loop_ip_pool_arc = ip_pool_arc.clone();

        task::spawn(async move {
            while let Some(packet) = tun_stream.next().await {
                match packet {
                    Ok(pkt) => {
                        let result = Self::handle_packet(&mut remote, loop_ip_pool_arc.clone(), &pkt).await;
                        if let Err(err) = result {
                            log::error!("handle_packet error {:?}", err);
                        }
                    }
                    Err(err) => panic!("Cannot get packet from tun, error: {:?}", err),
                }
            }
        });

        Self {
            id_length: server_config.id_length,
            topic: server_config.topic,
            local_addr,
            ip_pool: ip_pool_arc,
            remote_recv,
            tun_sink,
        }
    }

    // tun -> mqtt
    async fn handle_packet(
        remote: &mut remote::Remote,
        ip_pool: Arc<Mutex<IpPool>>,
        packet: &TunPacket,
    ) -> Result<(), rumqttc::v5::ClientError> {
        let dest = Ipv4Header::from_slice(&packet.get_bytes())
            .ok()
            .map(|(ipv4_header, _)| Ipv4Addr::from(ipv4_header.destination));
        if let Some(d) = dest {
            let ip_pool = ip_pool.lock().await;
            if let Some(topic) = ip_pool.get_reverse(&d.into()) {
                remote.publish(&topic, packet.get_bytes().to_vec()).await?;
            } else {
                log::debug!("drop packet: no tunnel for {:?}", &d);
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
                log::debug!("packet parse failed {:?}", error);
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
        self.tun_sink.send(TunPacket::new(packet_with_updated_header)).await?;
        Ok(())
    }

    pub async fn run(&mut self) {
        loop {
            if let Some((_topic, id_message)) = self.remote_recv.recv().await {
                let (id, message) = id_message.split_at(self.id_length);
                if let Err(err) = self.handle_remote_message(id, message).await {
                    log::error!("handle_remote_message error {:?}", err);
                }
            } else {
                break;
            }
        }
    }
}
