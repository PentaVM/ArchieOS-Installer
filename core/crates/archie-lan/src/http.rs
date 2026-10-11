//! A small HTTP GET, for the two things read from devices: a UPnP description and a
//! Roku's state. Plain HTTP, to a LAN address only, with a ceiling on time and size: a
//! device on the network is not trusted to send a sensible amount of anything.

use crate::is_lan;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// The most a device may send back.
pub const MAX_BODY: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(3);
/// The longest one device is listened to, however slowly it sends.
const DEADLINE: Duration = Duration::from_secs(6);

/// `http://host:port/path`, taken apart. Only literal IP hosts: a name would be resolved
/// by whoever answers DNS, and could point anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanUrl {
    host: IpAddr,
    port: u16,
    path: String,
}

impl LanUrl {
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("http://")?;
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };
        let address: SocketAddr = match authority.parse() {
            Ok(address) => address,
            Err(_) => SocketAddr::new(authority.parse().ok()?, 80),
        };
        let safe = path.len() <= 512 && path.bytes().all(|b| b.is_ascii_graphic());
        (is_lan(address.ip()) && safe).then(|| Self { host: address.ip(), port: address.port(), path: path.to_string() })
    }

    pub fn host(&self) -> IpAddr {
        self.host
    }
}

/// The body of a `200 OK`, or why there is none.
pub fn get(url: &LanUrl) -> Result<Vec<u8>, String> {
    let address = SocketAddr::new(url.host, url.port);
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let request = format!("GET {} HTTP/1.0\r\nHost: {address}\r\nConnection: close\r\n\r\n", url.path);
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    // The socket's timeout bounds each read, not the answer: a device that sends a byte
    // every two seconds would otherwise be listened to for days.
    let until = Instant::now() + DEADLINE;
    let ceiling = MAX_BODY + 8 * 1024;
    let mut response = Vec::new();
    let mut chunk = [0u8; 8192];
    while response.len() < ceiling {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the device took too long to answer".into());
        }
        stream.set_read_timeout(Some(left.min(TIMEOUT))).map_err(|e| e.to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&chunk[..read]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                return Err("the device took too long to answer".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    body_of(&response)
}

/// The body of a response, when its status is 200 and it fits.
pub fn body_of(response: &[u8]) -> Result<Vec<u8>, String> {
    let end = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the answer has no headers")?;
    let head = String::from_utf8_lossy(&response[..end]);
    let status = head.lines().next().unwrap_or("");
    if status.split_whitespace().nth(1) != Some("200") {
        return Err(format!("the device answered {status:?}"));
    }
    let body = &response[end + 4..];
    if body.len() > MAX_BODY {
        return Err("the device sent more than it may".into());
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_trickles_its_answer_is_given_up_on() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let _ = peer.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
            // Each byte arrives inside the socket's own timeout; only the deadline ends it.
            while peer.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        let started = Instant::now();
        // Built by hand: the parser refuses loopback, as it should.
        let answer = get(&LanUrl { host: "127.0.0.1".parse().unwrap(), port, path: "/".into() });
        assert_eq!(answer, Err("the device took too long to answer".to_string()));
        assert!(started.elapsed() < DEADLINE + TIMEOUT);
    }

    #[test]
    fn only_addresses_on_the_lan_are_fetched_from() {
        let url = LanUrl::parse("http://192.168.1.40:8060/query/device-info").unwrap();
        assert_eq!((url.port, url.path.as_str()), (8060, "/query/device-info"));
        assert_eq!(LanUrl::parse("http://10.0.0.2").unwrap().path, "/");
        for refused in [
            "http://8.8.8.8/desc.xml",          // not the owner's network
            "http://tv.local:8060/",            // a name somebody else resolves
            "https://192.168.1.40/",            // not what these devices speak
            "http://192.168.1.40/a b",          // not a path
            "http://127.0.0.1:8060/",           // this machine is not a device on the LAN
        ] {
            assert_eq!(LanUrl::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_body_is_taken_only_from_a_200_that_fits() {
        assert_eq!(body_of(b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n<root/>").unwrap(), b"<root/>");
        assert!(body_of(b"HTTP/1.1 404 Not Found\r\n\r\nno").is_err());
        assert!(body_of(b"garbage").is_err());
        let mut flood = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
        flood.resize(flood.len() + MAX_BODY + 1, b'x');
        assert!(body_of(&flood).is_err());
    }
}
