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
use rumqttc::v5::mqttbytes::v5::PublishProperties;
use std::ops::Range;

// Context for receiving messsage from remote
struct RemoteIncomingContext {
    nth: usize,
    mqtt_client: Arc<mqtt::AsyncClient>,
    sender: mpsc::Sender<(String, Bytes)>,
    subs: Arc<Mutex<Vec<String>>>,
}

struct RemoteClient {
    mqttc: Arc<mqtt::AsyncClient>,
    alias_pool: Option<LookupPool<String, u16, Range<u16>>>, // alias we invented
}

// Used for sending message to remote
pub struct Remote {
    clients: Vec<RemoteClient>,
    subs: Arc<Mutex<Vec<String>>>,
}

impl Remote {
    pub fn new(
        broker_opts: &Vec<mqtt::MqttOptions>,
        topics: Vec<String>,
    ) -> (Self, mpsc::Receiver<(String, Bytes)>) {
        let (sender, receiver) = mpsc::channel(128);
        let subs = Arc::new(Mutex::new(topics));
        let mut remote = Self {
            clients: Vec::with_capacity(broker_opts.len()),
            subs: subs.clone(),
        };
        for (idx, opt) in broker_opts.iter().enumerate() {
            let (mqtt_client, mut event_loop) = mqtt::AsyncClient::new(opt.clone(), 128);
            let arc_mqtt_client = Arc::new(mqtt_client);

            let alias_pool = opt
                .topic_alias_max()
                .filter(|n| *n > 0)
                .map(|count| LookupPool::new(1..count));
            let remote_client = RemoteClient {
                mqttc: arc_mqtt_client.clone(),
                alias_pool,
            };

            let mut context = RemoteIncomingContext {
                nth: idx,
                mqtt_client: arc_mqtt_client,
                sender: sender.clone(),
                subs: subs.clone(),
            };
            task::spawn(async move {
                loop {
                    use mqtt::Event::Incoming;
                    match event_loop.poll().await {
                        Ok(Incoming(pkt)) => {
                            log::debug!("broker[{}] recv {:?}", idx, pkt);
                            Self::handle_packet(&mut context, pkt).await;
                        }
                        x => {
                            log::debug!("broker[{}] recv {:?}", idx, x);
                            continue;
                        }
                    };
                }
            });
            remote.clients.push(remote_client);
        }
        (remote, receiver)
    }

    async fn handle_packet(context: &mut RemoteIncomingContext, pkt: Packet) {
        use mqtt::mqttbytes::v5::{ConnAck, ConnectReturnCode::Success, Filter, Publish};
        match pkt {
            Packet::ConnAck(ConnAck { code: Success, session_present, .. }) => {
                if !session_present {
                    log::info!("broker[{}] !session_present", context.nth);
                    let subs_v = context.subs.lock().await;
                    let subs = subs_v.iter().map(|path| Filter {
                        path: path.clone(),
                        qos: QoS::AtMostOnce,
                        nolocal: false,
                        preserve_retain: false,
                        retain_forward_rule: Default::default(),
                    });
                    if let Err(err) = context.mqtt_client.subscribe_many(subs).await {
                        log::info!("broker[{}] subscribe_many {:?}", context.nth, err);
                    }
                }
            }
            Packet::ConnAck(ConnAck { code, .. }) => {
                panic!("Refused by broker: {:?}", code);
            }
            Packet::Publish(Publish { topic, payload, .. }) => {
                let topic_str = String::from_utf8(topic.to_vec())
                    .ok()
                    .filter(|n| n.len() > 0);
                if let Some(topic) = topic_str {
                    _ = context.sender.send((topic, payload)).await; // What if it's not ok?
                } else {
                    log::debug!("drop packet, non utf8 topic: {:?}", topic);
                }
            }
            _ => (),
        }
    }

    pub async fn subscribe(&self, topic: String) -> Result<(), mqtt::ClientError> {
        let mut subs = self.subs.lock().await;
        if subs.contains(&topic) { return Ok(()); };
        subs.push(topic.clone()); // Ensure sub on reconnect
        self.subscribe_ephemeral(topic).await
    }

    pub async fn subscribe_ephemeral(&self, topic: String) -> Result<(), mqtt::ClientError> {
        for (idx, client) in self.clients.iter().enumerate() {
            let res = client.mqttc.subscribe(topic.clone(), QoS::AtMostOnce).await;
            if !res.is_ok() || idx == self.clients.len() - 1 { return res }
        }
        unreachable!()
    }

    pub async fn unsubscribe(&self, topic: String) -> Result<(), mqtt::ClientError> {
        for (idx, client) in self.clients.iter().enumerate() {
            let res = client.mqttc.unsubscribe(topic.clone()).await;
            if !res.is_ok() || idx == self.clients.len() - 1 { return res }
        }
        unreachable!()
    }

    pub async fn publish(
        &mut self,
        topic: &String,
        payload: Vec<u8>,
    ) -> Result<(), mqtt::ClientError> {
        let clients_length = self.clients.len();
        for (idx, client) in self.clients.iter_mut().enumerate() {
            let mut properties: PublishProperties = Default::default();
            let topic_to_send = if let Some(ref mut pool) = client.alias_pool {
                let already_sent_alias = pool.contains(topic);
                let alias = pool.get_forward(topic);
                properties.topic_alias = Some(alias);
                if already_sent_alias { "" } else { topic }
            } else { // Alias not used
                topic
            };
            log::debug!(
                "pub {:?} props: {:?}",
                topic_to_send,
                properties
            );
            let res = client
                .mqttc
                .publish_with_properties(
                    topic_to_send,
                    QoS::AtMostOnce,
                    false,
                    payload.clone(),
                    properties,
                )
                .await;
            if res.is_ok() {
                return Ok(());
            }
            if idx == clients_length - 1 {
                return res;
            }
        }
        unreachable!()
    }
}
