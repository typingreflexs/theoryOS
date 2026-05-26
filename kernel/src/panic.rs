//! Kernel panic handler — prints location and message, then halts the CPU.

use core::fmt::Write;
use core::panic::PanicInfo;

use crate::arch;
use crate::console::Console;

/// Called by `#[panic_handler]` — logs to serial and spins forever.
pub fn handle(info: &PanicInfo) -> ! {
    Console::print("\n*** KERNEL PANIC ***\n");
    if let Some(location) = info.location() {
        let _ = writeln!(
            Console,
            "  at {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    }
    let _ = writeln!(Console, "  {}", info.message());
    arch::halt_forever()
}
