use core::time::Duration;
use env_logger;
use zika::config::read_from_default_location;
use zika::server::Server;

#[tokio::main]
async fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let config = read_from_default_location().expect("A proper config file");
    log::debug!("Config = {:?}", config);
    let _ = Server::from_config(config);
    loop { tokio::time::sleep(Duration::from_secs(1)).await; }
}
