use bytes::Bytes;
use log;
use rumqttc::v5::{
    self as mqtt,
    mqttbytes::{v5::Packet, QoS},
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{sync::mpsc, task};

use crate::lookup_pool::LookupPool;

struct RemoteClient {
    nth: usize,
    mqttc: Arc<mqtt::AsyncClient>,
    // alias: LookupPool<String, u16>,
}

pub struct Remote {
    clients: Vec<RemoteClient>,
    receiver: mpsc::Receiver<(String, Bytes)>,
    subs: Arc<Mutex<Vec<String>>>,
}

impl Remote {
    pub fn new(broker_opts: &Vec<mqtt::MqttOptions>, topic: String) -> Self {
        let (sender, receiver) = mpsc::channel(128);
        let subs = Arc::new(Mutex::new(vec![topic]));
        let mut remote = Self {
            clients: Vec::with_capacity(broker_opts.len()),
            subs: subs.clone(),
            receiver,
        };
        for (idx, opt) in broker_opts.iter().enumerate() {
            let (mqtt_client, mut event_loop) = mqtt::AsyncClient::new(opt.clone(), 128);
            let arc_mqtt_client = Arc::new(mqtt_client);
            let remote_client = RemoteClient {
                nth: idx,
                mqttc: arc_mqtt_client.clone(),
                // alias: LookupPool::new(0..opt.topic_alias_max().unwrap_or(2)),
            };
            let loop_sender = sender.clone();
            let loop_mqtt_client = arc_mqtt_client.clone();
            let loop_subs = subs.clone();
            task::spawn(async move {
                loop {
                    use mqtt::Event::Incoming;
                    match event_loop.poll().await {
                        Ok(Incoming(pkt)) => {
                            log::debug!("Received Incoming Packet {:?}", pkt);
                            Self::handle_packet(
                                &loop_mqtt_client,
                                idx,
                                loop_sender.clone(),
                                pkt,
                                loop_subs.clone(),
                            )
                            .await;
                        }
                        x => {
                            log::debug!("Received Other Packet {:?}", x);
                            continue;
                        }
                    };
                }
            });
            remote.clients.push(remote_client);
        }
        remote
    }

    async fn handle_packet(
        mqtt_client: &mqtt::AsyncClient,
        nth: usize,
        sender: mpsc::Sender<(String, Bytes)>,
        pkt: Packet,
        subs_m: Arc<Mutex<Vec<String>>>,
    ) {
        use mqtt::mqttbytes::v5::{ConnAck, ConnectReturnCode::Success, Filter, Publish};
        match pkt {
            Packet::ConnAck(ConnAck {
                code: Success,
                properties: Some(prop),
                session_present,
            }) => {
                // if let Some(alias_max) = prop.topic_alias_max {
                //     log::info!("broker[{}] alias resize({})", nth, alias_max);
                //     client.alias.resize(0..alias_max);
                // }
                if !session_present {
                    log::info!("broker[{}] !session_present", nth);
                    let subs_v = subs_m.lock().await;
                    let subs = subs_v.iter().map(|path| Filter {
                        path: path.clone(),
                        qos: QoS::AtMostOnce,
                        nolocal: false,
                        preserve_retain: false,
                        retain_forward_rule: Default::default(),
                    });
                    if let Err(err) = mqtt_client.subscribe_many(subs).await {
                        log::info!("broker[{}] subscribe_many {:?}", nth, err);
                    }
                }
            }
            Packet::ConnAck(ConnAck { code, .. }) => {
                panic!("Refused by broker: {:?}", code);
            }
            Packet::Publish(Publish { topic, payload, .. }) => {
                if let Ok(topic) = String::from_utf8(topic.to_vec()) {
                    _ = sender.send((topic, payload)).await; // What if it's not ok?
                } else {
                    log::debug!("drop packet, non utf8 topic: {:?}", topic);
                }
            }
            _ => (),
        }
    }

    pub async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), mqtt::ClientError> {
        if self.clients.len() == 1 {
            return self.clients[0]
                .mqttc
                .publish(topic, QoS::AtMostOnce, false, payload)
                .await;
        }
        for (idx, client) in self.clients.iter().enumerate() {
            let res = client
                .mqttc
                .publish(topic.clone(), QoS::AtMostOnce, false, payload.clone())
                .await;
            if res.is_ok() {
                return Ok(());
            }
            if idx == self.clients.len() - 1 {
                return res;
            }
        }
        unreachable!()
    }

    pub async fn recv(&mut self) -> (String, Bytes) {
        match self.receiver.recv().await {
            Some(pkt) => pkt,
            _ => unreachable!(),
        }
    }
}
