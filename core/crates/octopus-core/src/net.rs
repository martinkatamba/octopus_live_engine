//! IP networks, for allow and block lists and trusted proxies.

use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix_len: u8,
}

impl IpNet {
    pub fn parse(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();
        if let Some((ip_str, prefix_str)) = trimmed.split_once('/') {
            let addr: IpAddr = ip_str
                .parse()
                .map_err(|_| format!("Invalid IP address in CIDR: {}", trimmed))?;
            let prefix_len: u8 = prefix_str
                .parse()
                .map_err(|_| format!("Invalid prefix length in CIDR: {}", trimmed))?;
            let max_len = match addr {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            if prefix_len > max_len {
                return Err(format!("Prefix length {} exceeds {}", prefix_len, max_len));
            }
            Ok(Self { addr, prefix_len })
        } else {
            let addr: IpAddr = trimmed
                .parse()
                .map_err(|_| format!("Invalid IP address: {}", trimmed))?;
            let prefix_len = match addr {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            Ok(Self { addr, prefix_len })
        }
    }

    pub fn contains(&self, other: &IpAddr) -> bool {
        match (self.addr, other) {
            (IpAddr::V4(net_ip), IpAddr::V4(ip)) => {
                if self.prefix_len == 0 {
                    return true;
                }
                let net_u32 = u32::from(net_ip);
                let ip_u32 = u32::from(*ip);
                let mask = if self.prefix_len >= 32 {
                    u32::MAX
                } else {
                    !((1u32 << (32 - self.prefix_len)) - 1)
                };
                (net_u32 & mask) == (ip_u32 & mask)
            }
            (IpAddr::V6(net_ip), IpAddr::V6(ip)) => {
                if self.prefix_len == 0 {
                    return true;
                }
                let net_u128 = u128::from(net_ip);
                let ip_u128 = u128::from(*ip);
                let mask = if self.prefix_len >= 128 {
                    u128::MAX
                } else {
                    !((1u128 << (128 - self.prefix_len)) - 1)
                };
                (net_u128 & mask) == (ip_u128 & mask)
            }
            _ => false,
        }
    }
}

/// The client's IP as every host derives it. A proxy may name the client in
/// `X-Forwarded-For` (first entry) or `X-Real-IP`, but only a peer in
/// `trusted` is believed; anyone else could pick their own IP that way.
pub fn client_ip(peer: &str, forwarded_for: &str, real_ip: &str, trusted: &[IpNet]) -> String {
    let peer = peer.trim();
    if peer.is_empty() {
        return "127.0.0.1".to_string();
    }
    let Ok(peer_ip) = peer.parse::<IpAddr>() else {
        return peer.to_string();
    };
    if trusted.iter().any(|net| net.contains(&peer_ip)) {
        if let Some(first) = forwarded_for.split(',').next().map(str::trim) {
            if !first.is_empty() {
                return first.to_string();
            }
        }
        if !real_ip.trim().is_empty() {
            return real_ip.trim().to_string();
        }
    }
    peer.to_string()
}

#[cfg(test)]
mod client_ip_tests {
    use super::*;

    #[test]
    fn forwarded_headers_count_only_from_trusted_proxies() {
        let trusted = vec![IpNet::parse("10.0.0.0/8").unwrap()];
        assert_eq!(
            client_ip("203.0.113.5", "1.2.3.4", "", &trusted),
            "203.0.113.5"
        );
        assert_eq!(
            client_ip("10.1.1.1", "1.2.3.4, 10.1.1.1", "", &trusted),
            "1.2.3.4"
        );
        assert_eq!(client_ip("10.1.1.1", "", " 5.6.7.8 ", &trusted), "5.6.7.8");
        assert_eq!(client_ip("10.1.1.1", "", "", &trusted), "10.1.1.1");
        assert_eq!(client_ip("", "1.2.3.4", "", &trusted), "127.0.0.1");
        assert_eq!(client_ip("::1", "1.2.3.4", "", &[]), "::1");
    }
}
