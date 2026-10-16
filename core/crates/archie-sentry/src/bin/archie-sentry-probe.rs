//! Diagnostics only: no physical writes and no deserializable authority.
use archie_sentry::adapters::ipp::IppClient;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || args[1] != "ipp" {
        eprintln!("usage: archie-sentry-probe ipp <configured LAN printer URI>");
        std::process::exit(2);
    }
    let result = match IppClient::new(&args[2]) {
        Ok(client) => client.get_printer_attributes(1).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(snapshot) => println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).expect("typed snapshot encoding")
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
