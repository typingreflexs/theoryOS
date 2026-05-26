//! Limine boot protocol — parse bootloader responses into `BootInfo`.
//!
//! `limine.rs` holds the Limine request/response structs; `info.rs` normalizes
//! them into a stable kernel-facing boot description.

pub mod limine;
pub mod info;

pub use info::BootInfo;

use limine::{LimineBootInfo, LimineRequests};

/// Parse Limine responses and produce normalized boot information.
pub fn parse_limine(requests: &LimineRequests) -> Option<BootInfo> {
    let raw = LimineBootInfo::from_requests(requests)?;
    Some(BootInfo::from_limine(raw))
}
