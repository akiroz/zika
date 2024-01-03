use core::time::Duration;
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use rustls::{Certificate, PrivateKey, RootCertStore};
use serde::Deserialize;
use std::fs::{read_to_string, File};
use std::io::BufReader;
use std::net::Ipv4Addr;

#[derive(Deserialize, Clone, Debug)]
pub struct MqttOptions {
    pub keepalive_interval: Option<u64>,

    pub username: Option<String>,
    pub password: Option<String>,

    pub ca_file: Option<String>,
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
    let text = read_to_string("zika_config.json").map_err(ConfigError::IOError)?;
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

        if let Some(opts) = mqtt_options {
            options.set_keep_alive(Duration::new(opts.keepalive_interval.unwrap_or(60), 0));
            options.set_topic_alias_max(opts.topic_alias_max);

            if let (Some(u), Some(p)) = (&opts.username, &opts.password) {
                options.set_credentials(u, p);
            };

            if opts.ca_file.is_some() || (opts.cert_file.is_some() && opts.key_file.is_some()) {
                let mut root_cert_store = RootCertStore::empty();
                if let Some(ca_file_path) = opts.ca_file {
                    let ca_file = File::open(ca_file_path).unwrap();
                    let mut ca_buf_read = BufReader::new(ca_file);
                    let cas = rustls_pemfile::certs(&mut ca_buf_read);
                    for ca in cas.into_iter() {
                        root_cert_store
                            .add(&Certificate(ca.unwrap().to_vec()))
                            .unwrap();
                    }
                }

                let tls_config = match (opts.cert_file, opts.key_file) {
                    (Some(cert_file_path), Some(key_file_path)) => {
                        let cert_file = File::open(cert_file_path).unwrap();
                        let key_file = File::open(key_file_path).unwrap();
                        let mut cert_buf_read = BufReader::new(cert_file);
                        let mut key_buf_read = BufReader::new(key_file);

                        let certs = rustls_pemfile::certs(&mut cert_buf_read);
                        let certs_vec = certs
                            .into_iter()
                            .map(|c| Certificate(c.unwrap().to_vec()))
                            .collect();

                        let mut keys = rustls_pemfile::ec_private_keys(&mut key_buf_read);
                        let first_key = keys.next().unwrap();
                        let key = PrivateKey(first_key.unwrap().secret_sec1_der().to_vec());

                        rustls::ClientConfig::builder()
                            .with_safe_defaults()
                            .with_root_certificates(root_cert_store)
                            .with_client_auth_cert(certs_vec, key)
                            .unwrap()
                    }
                    _ => rustls::ClientConfig::builder()
                        .with_safe_defaults()
                        .with_root_certificates(root_cert_store)
                        .with_no_client_auth(),
                };

                options.set_transport(rumqttc::Transport::tls_with_config(tls_config.into()));
            }
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
