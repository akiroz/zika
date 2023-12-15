use ipnetwork::{Ipv4Network, Ipv4NetworkIterator};
use std::net::Ipv4Addr;

pub struct SizedIpv4NetworkIterator {
    inner: Ipv4NetworkIterator,
    lowest: Ipv4Addr,
    highest: Ipv4Addr,
    ip: Ipv4Addr,
    size: u32,
}

impl SizedIpv4NetworkIterator {
    pub fn new(net: Ipv4Network) -> Self {
        Self {
            inner: net.iter(),
            size: net.size(),
            lowest: net.network(),
            highest: net.broadcast(),
            ip: net.ip(),
        }
    }
}

impl Iterator for SizedIpv4NetworkIterator {
    type Item = Ipv4Addr;

    fn next(&mut self) -> Option<Ipv4Addr> {
        // Return only usable IP
        let v = self.inner.next();
        if v == Some(self.lowest) || v == Some(self.ip) {
            self.next()
        } else if v == Some(self.highest) {
            None
        } else {
            v
        }
    }
}

impl ExactSizeIterator for SizedIpv4NetworkIterator {
    fn len(&self) -> usize {
        // Removing the first and the last ip as they are not usable
        // Removing local address
        (self.size - 3) as usize
    }
}
