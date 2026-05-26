//! Wi-Fi / network status.

use alloc::string::String;
use spin::Mutex;

use super::dhcp;
use super::http;

#[derive(Clone, Copy, Debug, Default)]
pub struct WifiStatus {
    pub hardware_present: bool,
    pub driver_loaded: bool,
    pub connected: bool,
}

static STATUS: Mutex<WifiStatus> = Mutex::new(WifiStatus {
    hardware_present: false,
    driver_loaded: true,
    connected: false,
});

pub fn init() {}

pub fn refresh() {
    let online = http::is_online();
    let mut st = STATUS.lock();
    st.hardware_present = false;
    st.connected = online;
}

pub fn status() -> WifiStatus {
    refresh();
    *STATUS.lock()
}

pub fn status_line() -> String {
    refresh();
    if let Some(ip) = dhcp::leased_ip() {
        return alloc::format!(
            "Internet: online via Ethernet {}.{}.{}.{}",
            ip.0[0],
            ip.0[1],
            ip.0[2],
            ip.0[3]
        );
    }
    String::from("Internet: offline — open Settings to connect")
}

pub fn ethernet_line() -> String {
    if let Some(ip) = dhcp::leased_ip() {
        alloc::format!(
            "Ethernet: {}.{}.{}.{} (DHCP)",
            ip.0[0],
            ip.0[1],
            ip.0[2],
            ip.0[3]
        )
    } else {
        String::from("Ethernet: not connected")
    }
}

pub fn sysfs_state() -> String {
    if http::is_online() {
        String::from("up\nethernet\n")
    } else {
        String::from("down\nno link\n")
    }
}

pub fn scan_results() -> &'static [&'static str] {
    if http::is_online() {
        &["(use Ethernet — no Wi-Fi adapter in this PC)"]
    } else {
        &["(network offline)"]
    }
}

pub fn wifi_line() -> &'static str {
    "Wi-Fi: no 802.11 adapter detected"
}
