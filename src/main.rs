use tnt_delivery_bot::config;

fn main() {
    println!("tnt-delivery-bot");
    let _ = config::Config::from_env();
}
