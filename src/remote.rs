use log;
use bytes::Bytes;
use tokio::{task, sync::mpsc};
use rumqttc::v5::{self as mqtt, mqttbytes::{v5::Packet, QoS}};

use crate::lookup_pool::LookupPool;

struct RemoteClient {
    nth: usize,
    mqttc: mqtt::AsyncClient,
    mqttev: mqtt::EventLoop,
    alias: LookupPool<String, u16>,
    sender: mpsc::Sender<(String, Bytes)>,
}

struct Remote {
    clients: Vec<RemoteClient>,
    receiver: mpsc::Receiver<(String, Bytes)>,
    subs: Vec<String>,
}

impl Remote {
    pub fn new(broker_opts: &Vec<mqtt::MqttOptions>) -> Self {
        let (sender, receiver) = mpsc::channel(128);
        let mut remote = Self {
            clients: Vec::with_capacity(broker_opts.len()),
            subs: Vec::new(),
            receiver,
        }; 
        for (idx, opt) in broker_opts.iter().enumerate() {
            let (mqtt_client, event_loop) = mqtt::AsyncClient::new(opt.clone(), 128);
            let mut remote_client = RemoteClient {
                nth: idx,
                mqttc: mqtt_client,
                mqttev: event_loop,
                alias: LookupPool::new(0..opt.topic_alias_max().unwrap_or(0)),
                sender: sender.clone(),
            };
            task::spawn(async move {
                loop {
                    use mqtt::Event::Incoming;
                    match remote_client.mqttev.poll().await {
                        Ok(Incoming(pkt)) => Self::handle_packet(&mut remote_client, pkt),
                        _ => continue,
                    };
                }
            });
            remote.clients.push(remote_client);
        }
        remote
    }

    async fn handle_packet(client: &mut RemoteClient, pkt: Packet, subs: &Vec<String>) {
        use mqtt::mqttbytes::v5::{ConnAck, Publish, Filter, ConnectReturnCode::Success};
        match pkt {
            Packet::ConnAck(ConnAck { code: Success, properties: Some(prop), session_present }) => {
                if let Some(alias_max) = prop.topic_alias_max {
                    log::info!("broker[{}] alias resize({})", client.nth, alias_max);
                    client.alias.resize(0..alias_max);
                }
                if !session_present {
                    log::info!("broker[{}] !session_present", client.nth);
                    let subs = subs.iter().map(|path| Filter {
                        path: path.clone(),
                        qos: QoS::AtMostOnce,
                        nolocal: false,
                        preserve_retain: false,
                        retain_forward_rule: Default::default(),
                    });
                    if let Err(err) = client.mqttc.subscribe_many(subs).await {
                        log::info!("broker[{}] subscribe_many {:?}", client.nth, err);

                    } 
                }
            },
            Packet::ConnAck(ConnAck { code, .. }) => {
                panic!("Refused by broker: {:?}", code);
            },
            Packet::Publish(Publish { topic, payload, .. }) => {
                if let Ok(topic) = String::from_utf8(topic.to_vec()) {
                    let _ = client.sender.send((topic, payload));
                } else {
                    log::debug!("drop packet, non utf8 topic: {:?}", topic);
                }
            },
            _ => ()
        }
    }

    pub async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), mqtt::ClientError> {
        if self.clients.len() == 1 {
            return self.clients[0].mqttc.publish(topic, QoS::AtMostOnce, false, payload).await
        }
        for (idx, client) in self.clients.iter().enumerate() {
            let res = client.mqttc.publish(topic.clone(), QoS::AtMostOnce, false, payload.clone()).await;
            if res.is_ok() { return Ok(()) }
            if idx == self.clients.len() - 1 { return res }
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