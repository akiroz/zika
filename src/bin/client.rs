use env_logger;
use zika::client::Client;
use zika::config::read_from_default_location;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let config = read_from_default_location().expect("A proper config file");
    log::debug!("Config = {:?}", config);
    let mut client = Client::from_config(config).await;
    client.run().await;
}
