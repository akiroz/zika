use core::time::Duration;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::fs;
use std::net::Ipv4Addr;

#[derive(Deserialize, Clone, Debug)]
pub struct MqttOptions {
    pub keepalive_interval: Option<u64>,

    pub username: Option<String>,
    pub password: Option<String>,

    pub ca_file: Option<String>,

    pub tls_insecure: Option<bool>,
    pub key_file: Option<String>,
    pub cert_file: Option<String>,

    pub topic_alias_max: Option<u16>,
}

#[derive(Deserialize, Debug)]
pub struct MqttBroker {
    pub options: Option<MqttOptions>,
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Debug)]
pub struct MqttConfig {
    pub options: Option<MqttOptions>,
    pub brokers: Vec<MqttBroker>,
}

#[derive(Deserialize, Debug)]
pub struct ServerConfig {
    #[serde(default = "default_id_length")]
    pub id_length: usize,

    pub topic: String,
    pub bind_cidr: String,
}

fn default_id_length() -> usize {
    return 4;
}

#[derive(Deserialize, Debug)]
pub struct ClientTunnel {
    #[serde(default = "default_id_length")]
    pub id_length: usize,

    pub topic: String,
    pub bind_addr: Ipv4Addr,
}

#[derive(Deserialize, Debug)]
pub struct ClientConfig {
    pub bind_cidr: String,
    pub tunnels: Vec<ClientTunnel>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub server: Option<ServerConfig>,
    pub client: Option<ClientConfig>,
}

#[derive(Debug)]
pub enum ConfigError {
    IOError(std::io::Error),
    DecodeError(serde_json::Error),
}

pub fn read_from_default_location() -> Result<Config, ConfigError> {
    let text = fs::read_to_string("zika_config.json").map_err(ConfigError::IOError)?;
    let deserialized: Config = serde_json::from_str(&text).map_err(ConfigError::DecodeError)?;
    return Ok(deserialized);
}

impl MqttOptions {
    pub fn merge_with_option(&self, another_option: MqttOptions) -> MqttOptions {
        MqttOptions {
            keepalive_interval: another_option
                .keepalive_interval
                .or(self.keepalive_interval),

            username: another_option.username.or(self.username.clone()),
            password: another_option.password.or(self.password.clone()),

            ca_file: another_option.ca_file.or(self.ca_file.clone()),

            tls_insecure: another_option.tls_insecure.or(self.tls_insecure),
            key_file: another_option.key_file.or(self.key_file.clone()),
            cert_file: another_option.cert_file.or(self.cert_file.clone()),

            topic_alias_max: another_option.topic_alias_max.or(self.topic_alias_max),
        }
    }
}

impl MqttBroker {
    pub fn to_mqtt_options(
        &self,
        base_mqtt_options: &Option<MqttOptions>,
    ) -> rumqttc::v5::MqttOptions {
        let mut rng = thread_rng();
        let random_id: String = (&mut rng)
            .sample_iter(Alphanumeric)
            .take(7)
            .map(char::from)
            .collect();
        let mut options = rumqttc::v5::MqttOptions::new(random_id, &self.host, self.port);
        let mqtt_options = match (base_mqtt_options, self.options.clone()) {
            (Some(b), Some(opts)) => Some(b.merge_with_option(opts)),
            (None, o) => o,
            (o, None) => o.clone(),
        };
        // TODO: more options
        if let Some(opts) = mqtt_options {
            options.set_keep_alive(Duration::new(opts.keepalive_interval.unwrap_or(60), 0));
            options.set_topic_alias_max(opts.topic_alias_max);

            if let (Some(u), Some(p)) = (&opts.username, &opts.password) {
                options.set_credentials(u, p);
            };
        };
        return options;
    }
}

impl Config {
    pub fn broker_mqtt_options(&self) -> Vec<rumqttc::v5::MqttOptions> {
        let base_options = &self.mqtt.options;
        return self
            .mqtt
            .brokers
            .iter()
            .map(|s| s.to_mqtt_options(base_options))
            .collect::<Vec<_>>();
    }
}
