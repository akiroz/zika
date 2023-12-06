use crate::config;
use crate::remote;

pub struct Server {
    remote: remote::Remote,
}

impl Server {
    pub fn new(config: config::Config) -> Self {
        let mqtt_options = config.broker_mqtt_options();
        let server_config = config.server.expect("Server config to be non-null");

        let remote = remote::Remote::new(&mqtt_options, vec![server_config.topic]);
        Self { remote: remote }
    }

    pub async fn run(&mut self) {
        loop {
            let x = self.remote.recv().await;
            log::debug!("recv = {:?}", x);
        }
    }
}
