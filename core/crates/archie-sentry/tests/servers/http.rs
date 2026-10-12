//! A small HTTP/1.1 server for the HTTP-based protocol servers: persistent
//! connections, Content-Length bodies, one handler over shared state.
//! Requests are logged as `METHOD /path` so tests can assert what crossed.
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const MAX_REQUEST: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}
impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type: Some(content_type.into()),
            body: body.into(),
        }
    }
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
        }
    }
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::new(status, "application/json", value.to_string())
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

pub struct HttpServer {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub base: String,
    pub log: Arc<Mutex<Vec<String>>>,
}
impl HttpServer {
    pub async fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let l = log.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (handler, log) = (handler.clone(), l.clone());
                tokio::spawn(async move { connection(stream, handler, log).await });
            }
        });
        Self { base, log }
    }
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

async fn connection(mut stream: TcpStream, handler: Handler, log: Arc<Mutex<Vec<String>>>) {
    let mut buffer = Vec::new();
    loop {
        let request = loop {
            if let Some(request) = parse(&mut buffer) {
                break request;
            }
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
            if buffer.len() > MAX_REQUEST {
                return;
            }
        };
        log.lock()
            .unwrap()
            .push(format!("{} {}", request.method, request.path));
        let response = handler(&request);
        let reason = match response.status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            _ => "Status",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
            response.status,
            response.body.len()
        );
        if let Some(kind) = &response.content_type {
            head.push_str(&format!("Content-Type: {kind}\r\n"));
        }
        head.push_str("\r\n");
        if stream.write_all(head.as_bytes()).await.is_err()
            || stream.write_all(&response.body).await.is_err()
        {
            return;
        }
    }
}

/// One complete request off the front of the buffer, if there is one.
fn parse(buffer: &mut Vec<u8>) -> Option<Request> {
    let end = buffer.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if buffer.len() < end + 4 + length {
        return None;
    }
    let body = buffer[end + 4..end + 4 + length].to_vec();
    buffer.drain(..end + 4 + length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}
