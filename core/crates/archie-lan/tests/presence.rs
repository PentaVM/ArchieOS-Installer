use archie_lan::presence::{probe_ports, Subnet};
use std::net::{Ipv4Addr, TcpListener};
use std::time::Duration;

#[test]
fn private_subnets_are_normalized_bounded_and_never_include_public_hosts() {
    let subnet: Subnet = "192.168.7.23/24".parse().unwrap();
    let hosts = subnet.hosts();
    assert_eq!(hosts.len(), 254);
    assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 7, 1));
    assert_eq!(hosts[253], Ipv4Addr::new(192, 168, 7, 254));
    for invalid in [
        "192.168.0.0/16",
        "8.8.8.8/32",
        "0.0.0.0/0",
        "10.0.0.0/33",
        "10.0.0.0",
        "127.0.0.1/32",
    ] {
        assert!(invalid.parse::<Subnet>().is_err(), "accepted {invalid}");
    }
    assert_eq!(
        "10.0.0.1/32".parse::<Subnet>().unwrap().hosts(),
        [Ipv4Addr::new(10, 0, 0, 1)]
    );
}

#[test]
fn an_open_tcp_port_is_detected_without_sending_print_or_control_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept = std::thread::spawn(move || {
        use std::io::Read;
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut bytes = [0u8; 1];
        assert_eq!(
            stream.read(&mut bytes).unwrap(),
            0,
            "probe sent application bytes"
        );
    });
    assert_eq!(
        probe_ports(Ipv4Addr::LOCALHOST, &[port], Duration::from_secs(1)),
        [port]
    );
    accept.join().unwrap();
}
