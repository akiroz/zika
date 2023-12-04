use env_logger;
use zika::client::Client;
use zika::config::read_from_default_location;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = read_from_default_location().unwrap();
    log::debug!("Config = {:?}", config);
    let mut client = Client::new(&config);
    client.run().await;
}
