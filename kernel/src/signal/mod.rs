//! POSIX signal delivery — pending queue, handler frames, sigreturn.
//!
//! Signals are checked after each syscall; `delivery.rs` builds user trampolines.

pub mod delivery;
pub mod types;

pub use delivery::{deliver_pending, fork_state, has_pending, init, kill, set_action, set_blocked, sigreturn};
pub use types::*;
