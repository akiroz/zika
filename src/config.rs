use core::time::Duration;
use std::error::Error as StdError;
use std::fs::{read_to_string, File};
use std::io::{self, BufReader, ErrorKind};
use std::net::Ipv4Addr;

use rand::{thread_rng, Rng, distributions::Alphanumeric};
use rustls::{Certificate, PrivateKey, RootCertStore};
use rumqttc::TlsConfiguration;
use toml;
use serde::Deserialize;
use serde_default_utils::*;

#[derive(Deserialize, Clone, Debug)]
pub struct MqttBroker {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Clone, Debug)]
pub struct MqttConfig {
    pub ca_file: Option<String>,
    pub key_file: Option<String>,
    pub cert_file: Option<String>,

    #[serde(default = "default_u64::<60>")]
    pub keepalive_interval: u64,
    #[serde(default = "default_u16::<255>")]
    pub topic_alias_max: u16,

    pub brokers: Vec<MqttBroker>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ServerConfig {
    #[serde(default = "default_usize::<4>")]
    pub id_length: usize,
    pub topic: String,
    pub bind_cidr: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ClientTunnel {
    #[serde(default = "default_usize::<4>")]
    pub id_length: usize,
    pub topic: String,
    pub bind_addr: Ipv4Addr,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ClientConfig {
    pub bind_cidr: String,
    pub tunnels: Vec<ClientTunnel>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub server: Option<ServerConfig>,
    pub client: Option<ClientConfig>,
}

#[derive(Debug)]
pub enum ConfigError {
    IOError(std::io::Error),
    DecodeError(toml::de::Error),
}

pub fn read_from_default_location() -> Result<Config, ConfigError> {
    let text = read_to_string("zika_config.toml").map_err(ConfigError::IOError)?;
    let deserialized: Config = toml::from_str(&text).map_err(ConfigError::DecodeError)?;
    return Ok(deserialized);
}



impl MqttConfig {
    pub fn to_mqtt_options(&self) -> Result<Vec<rumqttc::v5::MqttOptions>, Box<dyn StdError>> {
        let mut rng = thread_rng();
        let mut mqtt_options: Vec<rumqttc::v5::MqttOptions> = Vec::new();
        for broker_cfg in self.brokers.clone().into_iter() {
            let random_id: String = (0..7).map(|_| rng.sample(Alphanumeric) as char).collect();
            let mut opts = rumqttc::v5::MqttOptions::new(random_id, &broker_cfg.host, broker_cfg.port);
            opts.set_keep_alive(Duration::new(self.keepalive_interval, 0));
            if self.topic_alias_max > 0 {
                opts.set_topic_alias_max(Some(self.topic_alias_max));
            }
            let tls_builder = if let Some(ca_file) = self.ca_file.clone() {
                let mut ca_store = RootCertStore::empty();
                let mut ca_rd = BufReader::new(File::open(ca_file)?);
                for ca in rustls_pemfile::certs(&mut ca_rd) {
                    ca_store.add(&Certificate(ca?.to_vec()))?;
                }
                let builder = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(ca_store);
                Some(builder)
            } else {
                None
            };
            match (tls_builder, self.cert_file.clone()) {
                (Some(builder), Some(cert_file)) => {
                    let mut cert_rd = BufReader::new(File::open(cert_file)?);
                    let mut certs: Vec<Certificate> = Vec::new();
                    for der in rustls_pemfile::certs(&mut cert_rd) {
                        certs.push(Certificate(der?.to_vec()));
                    }
                    let mut key_rd = BufReader::new(File::open(self.key_file.clone().expect("non-null client key"))?);
                    let mut key_result = rustls_pemfile::ec_private_keys(&mut key_rd);
                    let sec1_key = key_result.next().unwrap_or_else(|| Err(io::Error::new(ErrorKind::NotFound, "No EC key")))?;
                    let key = PrivateKey(sec1_key.secret_sec1_der().to_vec());
                    let tls_cfg: TlsConfiguration = builder.with_client_auth_cert(certs, key)?.into();
                    opts.set_transport(rumqttc::Transport::tls_with_config(tls_cfg.into()));
                }
                (Some(builder), None) => {
                    let tls_cfg: TlsConfiguration = builder.with_no_client_auth().into();
                    opts.set_transport(rumqttc::Transport::tls_with_config(tls_cfg));
                }
                _ => ()
            }
            mqtt_options.push(opts);
        }
        Ok(mqtt_options)
    }
}