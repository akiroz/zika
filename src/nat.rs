use std::error::Error as StdError;
use std::io::{Cursor, Write as _};
use std::net::Ipv4Addr;
use etherparse::{Ipv4Header, TcpHeader, UdpHeader};

pub fn do_nat(pkt: &[u8], src: Ipv4Addr, dst: Ipv4Addr) -> Result<Vec<u8>, Box<dyn StdError>> {
    let mut cursor = Cursor::new(Vec::with_capacity(pkt.len()));
    
    // NOTE: IPv6 not supported currently
    let (mut ip_hdr, ip_body) = Ipv4Header::from_slice(&pkt)?;
    ip_hdr.source = src.octets();
    ip_hdr.destination = dst.octets();
    // NOTE: write method calculates checksum automatically
    ip_hdr.write(&mut cursor)?;
    
    if ip_hdr.protocol == 6 { // TCP
        let (mut tcp_hdr, tcp_body) = TcpHeader::from_slice(ip_body)?;
        tcp_hdr.checksum = tcp_hdr.calc_checksum_ipv4(&ip_hdr, tcp_body)?;
        tcp_hdr.write(&mut cursor)?;
        cursor.write_all(tcp_body)?;
    } else if ip_hdr.protocol == 17 { // UDP
        let (mut udp_hdr, udp_body) = UdpHeader::from_slice(ip_body)?;
        udp_hdr.checksum = udp_hdr.calc_checksum_ipv4(&ip_hdr, udp_body)?;
        udp_hdr.write(&mut cursor)?;
        cursor.write_all(udp_body)?;
    } else {
        cursor.write_all(ip_body)?;
    }

    Ok(cursor.into_inner())
}