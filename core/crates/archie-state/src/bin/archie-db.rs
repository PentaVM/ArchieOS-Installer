//! Native operator entry point. Credentials are read from a private file only.
use std::path::Path;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.as_slice() {
        [_, command, config] if command == "check" => archie_state::StateStore::open(config)
            .map(|_| "PostgreSQL/TimescaleDB/pgvector ready".into()),
        [_, command, config, output] if command == "snapshot" => {
            archie_state::snapshots::capture(Path::new(config), Path::new(output))
                .and_then(|v| Ok(serde_json::to_string_pretty(&v)?))
        }
        [_, command, config, output] if command == "restore-verify" => {
            archie_state::snapshots::restore_verify(Path::new(config), Path::new(output))
                .and_then(|v| Ok(serde_json::to_string_pretty(&v)?))
        }
        _ => {
            eprintln!("usage: archie-db check <private-database.json> | <snapshot|restore-verify> <private-database.json> <snapshot-directory>");
            std::process::exit(64);
        }
    };
    match result {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("archie-db: {e}");
            std::process::exit(1);
        }
    }
}
