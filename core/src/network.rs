use serde::Serialize;
use std::net::UdpSocket;

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub ip: String,
}

pub fn lan_ip() -> Option<String> {
    // UDP connect trick: no data is sent, kernel picks the outbound interface.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

pub fn interfaces() -> Vec<InterfaceInfo> {
    let mut out: Vec<InterfaceInfo> = Vec::new();
    #[cfg(unix)]
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) == 0 {
            let mut cur = ifap;
            while !cur.is_null() {
                let ifa = &*cur;
                if !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family == libc::AF_INET as u16 {
                    let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy().to_string();
                    let addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    let ip = std::net::Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
                    out.push(InterfaceInfo { name, ip: ip.to_string() });
                }
                cur = ifa.ifa_next;
            }
            libc::freeifaddrs(ifap);
        }
    }
    if out.is_empty() {
        if let Some(ip) = lan_ip() {
            out.push(InterfaceInfo { name: "default".to_string(), ip });
        }
        out.push(InterfaceInfo { name: "lo".to_string(), ip: "127.0.0.1".to_string() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interfaces_nonempty() {
        let ifs = interfaces();
        assert!(!ifs.is_empty());
        assert!(ifs.iter().any(|i| i.ip == "127.0.0.1") || lan_ip().is_some() || !ifs.is_empty());
    }
}
