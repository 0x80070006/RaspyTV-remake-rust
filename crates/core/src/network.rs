//! État du réseau : réseau local présent, Internet joignable.

use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct NetworkStatus {
    pub online: bool,
    pub lan: bool,
}

pub fn lan_available() -> bool {
    if_addrs::get_if_addrs().map(|list| list.iter().any(|iface| !iface.is_loopback() && iface.ip().is_ipv4())).unwrap_or(false)
}

pub fn status() -> NetworkStatus {
    let lan = lan_available();
    let agent = crate::http::agent(Duration::from_millis(2500));
    let online =
        agent.get("https://connectivitycheck.gstatic.com/generate_204").call().map(|r| r.status().as_u16() == 204 || r.status().is_success()).unwrap_or(false);
    NetworkStatus { online, lan }
}
