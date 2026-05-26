use crate::boot::BootInfo;
use crate::kernel_main;

pub fn kernel_entry() -> ! {
    let boot = BootInfo {
        hhdm_offset: 0,
        kernel_physical_base: 0,
        rsdp: None,
        memory_map: &[],
        cmdline: None,
        bsp_lapic_id: 0,
    };
    kernel_main(boot)
}
