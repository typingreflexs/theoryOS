use crate::boot::parse_limine;
use crate::boot::limine::LimineRequests;
use crate::console::serial::SerialPort;
use crate::kernel_main;

pub fn kernel_entry() -> ! {
    let requests = LimineRequests;
    let boot = parse_limine(&requests).expect("Limine failed to satisfy boot requests");

    SerialPort::init_early(boot.hhdm_offset);
    kernel_main(boot)
}
