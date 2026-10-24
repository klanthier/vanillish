//! Vanillish: **this machine's IPv4 addresses on its networks**, `(interface name, address)`,
//! loopback and interfaces that are down left out, in the system's order. On Windows the
//! adapter's friendly name ("Wi-Fi", "Ethernet", "vEthernet (WSL)"); empty elsewhere, where the
//! app asks `getifaddrs` itself (`benilla-app`'s `inilla::console`).

use std::net::Ipv4Addr;

/// The addresses, `(name, dotted quad)`.
pub fn ipv4_interfaces() -> Vec<(String, String)> {
    #[cfg(windows)]
    {
        crate::windows::net::ipv4_interfaces()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// An address another device on the network could reach this one at: not loopback (`127/8`),
/// not link-local (`169.254/16`, what an adapter gives itself when no DHCP answered), not
/// `0.0.0.0`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn reachable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_addresses_a_neighbour_can_reach_are_listed() {
        assert!(reachable(Ipv4Addr::new(192, 168, 1, 23)));
        assert!(reachable(Ipv4Addr::new(10, 0, 0, 5)));
        assert!(reachable(Ipv4Addr::new(172, 28, 160, 1)));
        assert!(!reachable(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!reachable(Ipv4Addr::new(127, 1, 2, 3)));
        assert!(!reachable(Ipv4Addr::new(169, 254, 10, 20)));
        assert!(!reachable(Ipv4Addr::UNSPECIFIED));
    }
}
