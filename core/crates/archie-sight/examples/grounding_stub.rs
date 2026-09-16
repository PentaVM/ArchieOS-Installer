//! A grounding model that is not one.
//!
//! It answers LocateAnything's contract with a box given on the command line, so the loop
//! around a model — capture, confirm, click, watch it redraw — can be measured and
//! exercised without a network, and so `archie-look` can be driven where no model runs.
//!
//! ```text
//! grounding_stub --port 8099 --model local/stub [--delay-ms 800] \
//!     --box "el botón Guardar=376,507,428,565" --box "el campo=376,446,624,503"
//! ```
//!
//! Boxes are in the 0–1000 coordinates the contract uses. The first `--box` whose text
//! appears in the request is the answer; with none matching, the first is used.
//! `--delay-ms` makes it take its time, which is how a loop is watched carrying on without
//! it. It grants nothing, decides nothing and keeps nothing: every request is answered and
//! dropped.

use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut boxes: Vec<(String, String)> = Vec::new();
    while let Some(at) = args.iter().position(|a| a == "--box") {
        if at + 1 >= args.len() {
            eprintln!("grounding_stub: --box needs <text>=x1,y1,x2,y2");
            return std::process::ExitCode::FAILURE;
        }
        let value = args.remove(at + 1);
        args.remove(at);
        match value.split_once('=') {
            Some((text, corners)) => boxes.push((text.to_string(), corners.to_string())),
            None => {
                eprintln!("grounding_stub: --box needs <text>=x1,y1,x2,y2");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    let mut flags: HashMap<String, String> = HashMap::new();
    while args.len() >= 2 {
        let name = args.remove(0);
        let value = args.remove(0);
        flags.insert(name, value);
    }
    let port = flags
        .get("--port")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8099);
    let slowly = flags
        .get("--delay-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let model = flags
        .get("--model")
        .cloned()
        .unwrap_or_else(|| "local/stub".to_string());
    if boxes.is_empty() {
        eprintln!("grounding_stub: at least one --box is needed");
        return std::process::ExitCode::FAILURE;
    }

    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("grounding_stub: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!(
        "grounding_stub: answering as {model} on http://{}/v1",
        listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default()
    );
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            continue;
        };
        // Each request on its own task: answering one at a time would turn a slow model
        // into a slow queue, which is not the thing being measured.
        let (boxes, model) = (boxes.clone(), model.clone());
        tokio::spawn(async move { answer(socket, boxes, model, slowly).await });
    }
}

/// One request: read it whole, wait if this was told to be slow, and answer with the box
/// whose text the request contains.
async fn answer(mut socket: TcpStream, boxes: Vec<(String, String)>, model: String, slowly: u64) {
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    // Headers, then as much body as Content-Length announced.
    loop {
        let read = socket.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    if slowly > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(slowly)).await;
    }
    let request = String::from_utf8_lossy(&bytes);
    let corners = boxes
        .iter()
        .find(|(text, _)| request.contains(text.as_str()))
        .map(|(_, corners)| corners.clone())
        .unwrap_or_else(|| boxes[0].1.clone());
    let answer = format!(
        "<box> {} </box>",
        corners
            .split(',')
            .map(|value| value.trim())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let body = format!(
        r#"{{"model":"{model}","choices":[{{"message":{{"role":"assistant","content":"{answer}"}},"finish_reason":"stop"}}]}}"#
    );
    let _ = socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
}
