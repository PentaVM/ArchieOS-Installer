use archie_installer::map_cache;
#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut source = None;
    let mut destination = None;
    let mut mirrors = Vec::new();
    let mut refresh = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => source = args.next(),
            "--destination" => destination = args.next(),
            "--mirror" => {
                if let Some(url) = args.next() {
                    mirrors.push(url)
                } else {
                    usage()
                }
            }
            "--refresh" => refresh = true,
            _ => usage(),
        }
    }
    let (Some(source), Some(destination)) = (source, destination) else {
        usage()
    };
    if let Err(error) = map_cache::fetch(
        &source,
        std::path::Path::new(&destination),
        &mirrors,
        refresh,
    )
    .await
    {
        eprintln!("map download stopped: {error}");
        std::process::exit(1);
    }
}
fn usage() -> ! {
    eprintln!("usage: archie-map-fetch --url <Geofabrik PBF> --destination <directory> [--mirror <HTTPS PBF>] [--refresh]");
    std::process::exit(64)
}
