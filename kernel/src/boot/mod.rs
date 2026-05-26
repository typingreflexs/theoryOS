pub mod limine;
pub mod info;

pub use info::BootInfo;

use limine::{LimineBootInfo, LimineRequests};

/// Parse Limine responses and produce normalized boot information.
pub fn parse_limine(requests: &LimineRequests) -> Option<BootInfo> {
    let raw = LimineBootInfo::from_requests(requests)?;
    Some(BootInfo::from_limine(raw))
}
