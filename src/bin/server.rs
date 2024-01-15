use env_logger;
use zika::config::read_from_default_location;
use zika::server::Server;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = read_from_default_location().expect("A proper config file");
    log::debug!("Config = {:?}", config);
    let mut server = Server::from_config(config);
    server.run().await;
}
