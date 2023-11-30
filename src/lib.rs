pub mod config;
mod lookup_pool;
mod remote;

pub struct Client {}

impl Client {}

pub struct Server {
    remote: remote::Remote,
}

impl Server {
    pub fn new(config: config::Config) -> Self {
        let mqtt_options = config.broker_mqtt_options();
        match config.server {
            Some(s) => Self {
                remote: remote::Remote::new(&mqtt_options, s.topic),
            },
            _ => panic!("Server config not found"),
        }
    }

    pub async fn run(&mut self) {
        loop {
            let x = self.remote.recv().await;
            log::debug!("recv = {:?}", x);
        }
    }
}
