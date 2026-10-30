//! Reverse proxies allowed to name the client in `X-Real-IP` (`--trusted-proxy`).

use std::net::IpAddr;
use std::sync::OnceLock;

/// Set once at startup; empty means only a loopback peer is trusted.
static TRUSTED: OnceLock<Vec<Cidr>> = OnceLock::new();

pub fn set_trusted(list: Vec<Cidr>) {
    let _ = TRUSTED.set(list);
}

pub fn trusted() -> &'static [Cidr] {
    TRUSTED.get().map(Vec::as_slice).unwrap_or(&[])
}

/// An address block such as `172.16.0.0/12` or `fd00::/8`. A bare IP is a single host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Option<Self> {
        let (ip, prefix) = match text.trim().split_once('/') {
            Some((ip, bits)) => (ip.parse::<IpAddr>().ok()?, Some(bits.parse::<u8>().ok()?)),
            None => (text.trim().parse::<IpAddr>().ok()?, None),
        };
        let max = if ip.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self { net: ip, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        fn masked(bits: u128, width: u32, prefix: u8) -> u128 {
            if prefix == 0 {
                0
            } else {
                bits >> (width - u32::from(prefix))
            }
        }
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                masked(u32::from(net).into(), 32, self.prefix)
                    == masked(u32::from(ip).into(), 32, self.prefix)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                masked(net.into(), 128, self.prefix) == masked(ip.into(), 128, self.prefix)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cidr;

    #[test]
    fn parses_and_matches_blocks() {
        let net = Cidr::parse("172.16.0.0/12").unwrap();
        assert!(net.contains("172.17.0.5".parse().unwrap()));
        assert!(!net.contains("172.32.0.1".parse().unwrap()));
        assert!(!net.contains("::1".parse().unwrap()));
        let host = Cidr::parse("10.0.0.7").unwrap();
        assert!(host.contains("10.0.0.7".parse().unwrap()));
        assert!(!host.contains("10.0.0.8".parse().unwrap()));
        assert!(
            Cidr::parse("0.0.0.0/0")
                .unwrap()
                .contains("8.8.8.8".parse().unwrap())
        );
        assert!(
            Cidr::parse("fd00::/8")
                .unwrap()
                .contains("fd12::1".parse().unwrap())
        );
        assert_eq!(Cidr::parse("10.0.0.0/33"), None);
        assert_eq!(Cidr::parse("nope/8"), None);
    }
}
