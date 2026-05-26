pub mod serial;

use core::fmt::{self, Write};

use spin::Mutex;

use serial::SerialPort;

struct ConsoleState {
    serial: SerialPort,
}

static CONSOLE: Mutex<Option<ConsoleState>> = Mutex::new(None);

pub struct Console;

impl Console {
    pub fn init() {
        let hhdm = crate::boot_info().hhdm_offset;
        SerialPort::init_early(hhdm);
        *CONSOLE.lock() = Some(ConsoleState {
            serial: SerialPort::new(hhdm),
        });
    }

    pub fn println(message: &str) {
        let _ = writeln!(Console, "{message}");
    }

    pub fn print(message: &str) {
        let _ = write!(Console, "{message}");
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(state) = CONSOLE.lock().as_mut() {
            state.serial.write_str(s)
        } else {
            Ok(())
        }
    }
}
