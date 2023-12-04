use env_logger;
use zika::config::read_from_default_location;
use zika::Client;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = read_from_default_location().unwrap();
    log::debug!("Config = {:?}", config);
    if let Some(client_config) = config.client {
        for tun_config in client_config.tunnels.iter() {
            let mut client = Client::new(tun_config, &config.driver);
            client.run().await;
        }
    }
}
