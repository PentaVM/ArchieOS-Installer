//! SSDP: the question UPnP devices answer ("who is here?"), and reading the answers.

use crate::is_lan;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const GROUP: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250)), 1900);
/// A device answers once for each service it offers; past this the listening stops early.
const MAX_ANSWERS: usize = 2048;

/// One device's answer to a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub from: IpAddr,
    /// Where the device publishes its description, when it does.
    pub location: Option<String>,
    /// What it says it is, e.g. `urn:schemas-upnp-org:device:MediaRenderer:1` or `roku:ecp`.
    pub target: String,
    /// The software it reports, e.g. `Roku/12.5.0 UPnP/1.0 Roku/12.5.0`.
    pub server: String,
    pub usn: String,
}

/// The search itself. `MX` is how many seconds a device may wait before answering, so
/// that a network full of them does not answer at once.
pub fn search_message(wait_seconds: u8) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {wait_seconds}\r\nST: ssdp:all\r\n\r\n"
    )
}

/// Reads one answer. `None` for anything that is not a `200 OK` to a search.
pub fn parse_answer(from: IpAddr, datagram: &[u8]) -> Option<Answer> {
    let text = std::str::from_utf8(datagram).ok()?;
    let mut lines = text.split("\r\n");
    let status = lines.next()?;
    if !(status.starts_with("HTTP/1.") && status.contains(" 200 ")) {
        return None;
    }
    let headers: BTreeMap<String, &str> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim()))
        .collect();
    let field = |name: &str| {
        headers
            .get(name)
            .copied()
            .unwrap_or("")
            .chars()
            .take(512)
            .collect::<String>()
    };
    Some(Answer {
        from,
        location: headers
            .get("location")
            .map(|url| url.chars().take(512).collect()),
        target: field("st"),
        server: field("server"),
        usn: field("usn"),
    })
}

/// Asks the network `local` is on and collects answers for `listen`. Answers from outside
/// the LAN are dropped: nothing off the owner's network is believed about what is on it.
pub fn search(local: Ipv4Addr, listen: Duration) -> std::io::Result<Vec<Answer>> {
    let socket = UdpSocket::bind((local, 0))?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    let message = search_message(listen.as_secs().clamp(1, 5) as u8);
    // Twice: SSDP is UDP, and one lost datagram would hide the whole network.
    for _ in 0..2 {
        socket.send_to(message.as_bytes(), GROUP)?;
    }
    let until = Instant::now() + listen;
    let mut answers = Vec::new();
    let mut buffer = [0u8; 2048];
    while Instant::now() < until && answers.len() < MAX_ANSWERS {
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) if is_lan(from.ip()) => {
                if let Some(answer) = parse_answer(from.ip(), &buffer[..length]) {
                    if !answers.contains(&answer) {
                        answers.push(answer);
                    }
                }
            }
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROKU: &str = "HTTP/1.1 200 OK\r\nCache-Control: max-age=3600\r\nST: roku:ecp\r\n\
                        USN: uuid:roku:ecp:YN00AB123456\r\nExt:\r\nServer: Roku/12.5.0 UPnP/1.0 Roku/12.5.0\r\n\
                        LOCATION: http://192.168.1.40:8060/\r\n\r\n";

    #[test]
    fn an_answer_is_read_whatever_case_its_headers_are_in() {
        let answer = parse_answer("192.168.1.40".parse().unwrap(), ROKU.as_bytes()).unwrap();
        assert_eq!(answer.target, "roku:ecp");
        assert_eq!(
            answer.location.as_deref(),
            Some("http://192.168.1.40:8060/")
        );
        assert!(answer.server.starts_with("Roku/"));
        assert_eq!(answer.usn, "uuid:roku:ecp:YN00AB123456");
    }

    #[test]
    fn what_is_not_an_answer_to_a_search_is_ignored() {
        let from: IpAddr = "192.168.1.40".parse().unwrap();
        // Another machine's own search, and a device announcing itself unasked.
        assert_eq!(parse_answer(from, search_message(2).as_bytes()), None);
        assert_eq!(
            parse_answer(from, b"NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\n\r\n"),
            None
        );
        assert_eq!(parse_answer(from, &[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn the_search_asks_everything_to_answer_within_the_wait() {
        let message = search_message(3);
        assert!(message.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(message.contains("MX: 3\r\n") && message.contains("ST: ssdp:all\r\n"));
        assert!(message.ends_with("\r\n\r\n"));
    }
}
