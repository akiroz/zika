use zika::Client;

use zika::config::read_from_default_location;

#[tokio::main]
async fn main() {
    let config = read_from_default_location();
    println!("deserialized = {:?}", config);
}
