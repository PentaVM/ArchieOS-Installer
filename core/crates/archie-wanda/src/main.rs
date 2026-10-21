use archie_wanda::{
    config::WandaConfig,
    server::{router_with_service, Service},
    store::WandaStore,
};
use clap::Parser;
#[derive(Parser)]
#[command(about = "Wanda context service over the existing Archie authorities")]
struct Args {
    #[arg(long)]
    config: std::path::PathBuf,
    #[arg(long, default_value = "127.0.0.1:8791")]
    listen: std::net::SocketAddr,
    /// Run coalesced recommendations every 30 seconds; no outgoing actions.
    #[arg(long)]
    proactive: bool,
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.listen.ip().is_loopback() {
        return Err("Wanda binds only to loopback; use an authenticated TLS reverse proxy for remote producers".into());
    }
    let service = Service::new(WandaStore::new(WandaConfig::load(&args.config)?)?);
    if args.proactive {
        let background = service.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_secs(30));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                background.refresh().await;
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    eprintln!("Wanda listening on {}", listener.local_addr()?);
    axum::serve(listener, router_with_service(service))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
