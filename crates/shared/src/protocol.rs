use std::net::{Ipv4Addr, SocketAddrV4};

/// Encode a destination address as a 6-byte wire header.
///
/// Layout: `[4 bytes IPv4 big-endian][2 bytes port big-endian]`
///
/// The data-plane sends this header to the control-plane at the start of every
/// proxied TCP connection so the control-plane knows the original destination.
pub fn encode_destination(addr: SocketAddrV4) -> [u8; 6] {
    let ip = addr.ip().octets();
    let port = addr.port().to_be_bytes();
    [ip[0], ip[1], ip[2], ip[3], port[0], port[1]]
}

/// Decode a 6-byte wire header back into a destination address.
pub fn decode_destination(header: [u8; 6]) -> SocketAddrV4 {
    let ip = Ipv4Addr::new(header[0], header[1], header[2], header[3]);
    let port = u16::from_be_bytes([header[4], header[5]]);
    SocketAddrV4::new(ip, port)
}
