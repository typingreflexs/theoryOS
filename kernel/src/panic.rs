use core::fmt::Write;
use core::panic::PanicInfo;

use crate::arch;
use crate::console::Console;

pub fn handle(info: &PanicInfo) -> ! {
    Console::print("\n*** KERNEL PANIC ***\n");
    if let Some(location) = info.location() {
        let _ = writeln!(
            Console,
            "At {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    }
    if let Some(message) = info.message().as_str() {
        let _ = writeln!(Console, "Message: {message}");
    } else {
        let _ = write!(Console, "Message: {}", info.message());
    }
    arch::halt_forever()
}
