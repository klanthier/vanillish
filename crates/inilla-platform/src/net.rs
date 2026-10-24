//! Vanillish: **this machine's IPv4 addresses on its networks**, `(interface name, address)`,
//! loopback and interfaces that are down left out, in the system's order. On Windows the
//! adapter's friendly name ("Wi-Fi", "Ethernet", "vEthernet (WSL)"); empty elsewhere, where the
//! app asks `getifaddrs` itself (`benilla-app`'s `inilla::console`).

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
