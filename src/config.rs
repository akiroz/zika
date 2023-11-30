use core::time::Duration;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize, Debug)]
pub struct MqttOptions {
    #[serde(default = "default_keepalive_interval")]
    pub keepalive_interval: u64,

    #[serde(default = "default_reconnect_interval_min")]
    pub reconnect_interval_min: u64,

    #[serde(default = "default_reconnect_interval_max")]
    pub reconnect_interval_max: u64,

    pub username: Option<String>,
    pub password: Option<String>,

    pub ca_file: Option<String>,

    #[serde(default = "default_tls_insecure")]
    pub tls_insecure: bool,
    pub key_file: Option<String>,
    pub cert_file: Option<String>,
}

fn default_keepalive_interval() -> u64 {
    return 60;
}

fn default_reconnect_interval_min() -> u64 {
    return 1;
}

fn default_reconnect_interval_max() -> u64 {
    return 60;
}

fn default_tls_insecure() -> bool {
    return false;
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
pub struct DriverPcap {
    pub interface: String,
}

#[derive(Deserialize, Debug)]
pub struct DriverTun {
    pub netmask: String,
}

#[derive(Deserialize, Debug)]
pub struct DriverConfig {
    pub local_addr: String,
    pub pcap: Option<DriverPcap>,
    pub tun: Option<DriverTun>,
}

#[derive(Deserialize, Debug)]
pub struct ServerConfig {
    #[serde(default = "default_id_length")]
    pub id_length: u8,

    pub topic: String,
    pub pool_start: String,
    pub pool_end: String,
}

fn default_id_length() -> u8 {
    return 4;
}

#[derive(Deserialize, Debug)]
pub struct ClientTunnel {
    #[serde(default = "default_id_length")]
    pub id_length: u8,

    pub topic: String,
    pub bind_addr: String,
}

#[derive(Deserialize, Debug)]
pub struct ClientConfig {
    pub tunnels: Vec<ClientTunnel>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub driver: DriverConfig,
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

impl MqttBroker {
    pub fn to_mqtt_options(&self) -> rumqttc::v5::MqttOptions {
        let mut rng = thread_rng();
        let random_id: String = (&mut rng)
            .sample_iter(Alphanumeric)
            .take(7)
            .map(char::from)
            .collect();
        let mut options = rumqttc::v5::MqttOptions::new(random_id, &self.host, self.port);
        options.set_topic_alias_max(Some(5));
        // TODO: more options
        match &self.options {
            None => {}
            Some(opts) => {
                options.set_keep_alive(Duration::new(opts.keepalive_interval, 0));

                match (&opts.username, &opts.password) {
                    (Some(u), Some(p)) => {
                        options.set_credentials(u, p);
                    }
                    _ => {}
                };
            }
        };
        return options;
    }
}

impl Config {
    pub fn broker_mqtt_options(&self) -> Vec<rumqttc::v5::MqttOptions> {
        return self
            .mqtt
            .brokers
            .iter()
            .map(|s| s.to_mqtt_options())
            .collect::<Vec<_>>();
    }
}