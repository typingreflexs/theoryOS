//! Kernel security subsystem — hardening, privileges, and isolation.

pub mod canary;
pub mod capability;
pub mod kpti;
pub mod seccomp;

use crate::console::Console;

pub use canary::{guard, verify_cpu_stack};
pub use capability::{CapSet, Credentials};
pub use kpti::{enabled as kpti_enabled, with_user_as};
pub use seccomp::{check_current as seccomp_allows, SeccompFilter, SECCOMP_MODE_FILTER, SECCOMP_MODE_STRICT};

pub fn capable(cap: CapSet) -> bool {
    crate::proc::capable(cap)
}

pub fn init() {
    kpti::init();
    canary::init();
    canary::export_guard_to_compiler();
    Console::println("[security] KPTI, stack canaries, capabilities, seccomp ready");
}

/// Verify NX is enforced on a mapping's page flags.
pub fn assert_nx_unless_exec(exec: bool, has_nx: bool) {
    if !exec && !has_nx {
        // NX bit must be set on non-executable pages when NXE is enabled.
        debug_assert!(false, "mapping missing NX bit");
    }
}
