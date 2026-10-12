//! A minimal blocking HTTP/1.1 GET client for the loopback projection.
//!
//! Hand-written for the same reason `archie-displayd`'s server is: one peer, a few GET
//! routes, no keep-alive, no TLS, no chunking. It adds no HTTP dependency to the build.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use thiserror::Error;

/// Largest response body accepted. A display snapshot is kilobytes; this bounds what a
/// misbehaving peer on the loopback port can make the shell buffer.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEAD_BYTES: usize = 16 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0} is not a loopback address; the shell bridge only talks to this machine")]
    NotLoopback(SocketAddr),
    #[error("could not connect: {0}")]
    Connect(std::io::Error),
    #[error("i/o error: {0}")]
    Io(std::io::Error),
    #[error("the answer was not a well-formed HTTP response")]
    Malformed,
    #[error("the server answered HTTP {0}")]
    BadStatus(u16),
    #[error("the answer exceeded {MAX_BODY_BYTES} bytes")]
    TooLarge,
    #[error("the answer was cut short ({got} of {declared} bytes)")]
    Truncated { got: usize, declared: usize },
}

/// A read-only client bound to one loopback address.
#[derive(Debug, Clone, Copy)]
pub struct Client {
    addr: SocketAddr,
}

impl Client {
    pub fn new(addr: SocketAddr) -> Result<Self, ClientError> {
        if !addr.ip().is_loopback() {
            return Err(ClientError::NotLoopback(addr));
        }
        Ok(Self { addr })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// GET `path` and return the body of a `200` answer.
    pub fn get(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        // The path is chosen by this crate, never by the shell; still, refuse anything that
        // could split the request line.
        if !path.starts_with('/') || path.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(ClientError::Malformed);
        }
        let mut stream = TcpStream::connect_timeout(&self.addr, CONNECT_TIMEOUT)
            .map_err(ClientError::Connect)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(ClientError::Io)?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n\r\n",
            self.addr
        );
        stream
            .write_all(request.as_bytes())
            .map_err(ClientError::Io)?;

        let mut raw = Vec::new();
        let limit = (MAX_HEAD_BYTES + MAX_BODY_BYTES + 1) as u64;
        stream
            .take(limit)
            .read_to_end(&mut raw)
            .map_err(ClientError::Io)?;
        parse_response(&raw)
    }
}

fn parse_response(raw: &[u8]) -> Result<Vec<u8>, ClientError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(ClientError::Malformed)?;
    if split > MAX_HEAD_BYTES {
        return Err(ClientError::Malformed);
    }
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| ClientError::Malformed)?;
    let body = &raw[split + 4..];

    let mut lines = head.lines();
    let status_line = lines.next().ok_or(ClientError::Malformed)?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().ok_or(ClientError::Malformed)?;
    if !version.starts_with("HTTP/") {
        return Err(ClientError::Malformed);
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(ClientError::Malformed)?;
    if status != 200 {
        return Err(ClientError::BadStatus(status));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ClientError::TooLarge);
    }
    let declared = lines.find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    if let Some(declared) = declared {
        if declared > MAX_BODY_BYTES {
            return Err(ClientError::TooLarge);
        }
        if body.len() < declared {
            return Err(ClientError::Truncated {
                got: body.len(),
                declared,
            });
        }
        return Ok(body[..declared].to_vec());
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_200_answer_yields_its_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(parse_response(raw).unwrap(), b"{}");
    }

    #[test]
    fn a_non_200_answer_is_an_error_not_a_body() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        assert!(matches!(
            parse_response(raw),
            Err(ClientError::BadStatus(404))
        ));
    }

    #[test]
    fn a_short_body_is_truncated_not_silently_accepted() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n{}";
        assert!(matches!(
            parse_response(raw),
            Err(ClientError::Truncated {
                got: 2,
                declared: 10
            })
        ));
    }

    #[test]
    fn text_that_is_not_http_is_malformed() {
        assert!(matches!(
            parse_response(b"not http at all\r\n\r\n"),
            Err(ClientError::Malformed)
        ));
        assert!(matches!(
            parse_response(b"no head terminator"),
            Err(ClientError::Malformed)
        ));
    }

    #[test]
    fn only_loopback_addresses_are_accepted() {
        assert!(Client::new("127.0.0.1:7666".parse().unwrap()).is_ok());
        assert!(Client::new("[::1]:7666".parse().unwrap()).is_ok());
        assert!(matches!(
            Client::new("192.168.1.10:7666".parse().unwrap()),
            Err(ClientError::NotLoopback(_))
        ));
        assert!(matches!(
            Client::new("0.0.0.0:7666".parse().unwrap()),
            Err(ClientError::NotLoopback(_))
        ));
    }

    #[test]
    fn a_path_that_could_split_the_request_line_is_refused() {
        let client = Client::new("127.0.0.1:1".parse().unwrap()).unwrap();
        assert!(matches!(
            client.get("/api/snapshot HTTP/1.1\r\nX: y"),
            Err(ClientError::Malformed)
        ));
        assert!(matches!(client.get("api"), Err(ClientError::Malformed)));
    }
}
