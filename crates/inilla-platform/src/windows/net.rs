//! `GetAdaptersAddresses` for [`crate::net::ipv4_interfaces`].

use std::net::Ipv4Addr;

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
    IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN, SOCKET_ADDRESS};

/// Every adapter that is up and not the loopback, each of its unicast IPv4 addresses a neighbour
/// can reach, in the order Windows lists them. Empty when the call fails.
pub(crate) fn ipv4_interfaces() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(buf) = adapters() else {
        return out;
    };
    let mut adapter = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    while !adapter.is_null() {
        // SAFETY: a record of the list `GetAdaptersAddresses` wrote into `buf`, which outlives
        // this loop; `Next` is null at its end.
        let a = unsafe { &*adapter };
        if a.OperStatus == IfOperStatusUp && a.IfType != IF_TYPE_SOFTWARE_LOOPBACK {
            // SAFETY: the friendly name is a NUL-terminated string inside `buf` (or null).
            let name = unsafe { wide_str(a.FriendlyName) };
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                // SAFETY: as for the adapter, a record of the same list.
                let u = unsafe { &*unicast };
                // SAFETY: the address points into `buf` for `iSockaddrLength` bytes.
                if let Some(ip) =
                    unsafe { ipv4(&u.Address) }.filter(|ip| crate::net::reachable(*ip))
                {
                    out.push((name.clone(), ip.to_string()));
                }
                unicast = u.Next;
            }
        }
        adapter = a.Next;
    }
    out
}

/// The adapter list, in a buffer aligned for its records: asked with Microsoft's suggested 15 KB
/// first, then as much as the call says it needs (an adapter may come between two calls).
fn adapters() -> Option<Vec<u64>> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 15 * 1024;
    for _ in 0..4 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: `buf` holds at least `size` bytes, 8-aligned as the records need; on overflow
        // the call writes the size it needs into `size` and nothing into `buf`.
        let rc = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET),
                flags,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match rc {
            ERROR_SUCCESS => return Some(buf),
            ERROR_BUFFER_OVERFLOW => continue,
            // ERROR_NO_DATA: no adapter has an IPv4 address.
            _ => return None,
        }
    }
    None
}

/// The IPv4 address in `sa`, if it is one.
///
/// # Safety
/// `sa.lpSockaddr` is null or points at `sa.iSockaddrLength` readable bytes.
unsafe fn ipv4(sa: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
    let len = usize::try_from(sa.iSockaddrLength).ok()?;
    if sa.lpSockaddr.is_null() || len < std::mem::size_of::<SOCKADDR_IN>() {
        return None;
    }
    // SAFETY: the caller's promise, and the length was checked; read unaligned, whatever the
    // record's packing.
    let sin = unsafe { std::ptr::read_unaligned(sa.lpSockaddr.cast::<SOCKADDR_IN>()) };
    if sin.sin_family != AF_INET {
        return None;
    }
    // SAFETY: every field of the `in_addr` union is the same four bytes, in network order.
    Some(Ipv4Addr::from(
        unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes(),
    ))
}

/// A NUL-terminated UTF-16 string as Rust's, empty for null.
///
/// # Safety
/// `p` is null or points at a NUL-terminated UTF-16 string.
unsafe fn wide_str(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: the caller's promise: every unit up to the NUL is readable.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: the `len` units just read.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}
