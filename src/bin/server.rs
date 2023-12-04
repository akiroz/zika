use env_logger;
use zika::config::read_from_default_location;
use zika::server::Server;

#[tokio::main]
async fn main() {
    env_logger::init();
    let config = read_from_default_location().unwrap();
    log::debug!("Config = {:?}", config);
    let mut server = Server::new(config);
    server.run().await;
}
