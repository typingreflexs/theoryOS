pub mod x86_64;

pub use x86_64::{PageFlags, PageTable, PhysFrame};

use spin::Once;

static INIT: Once<()> = Once::new();

pub fn init() {
    INIT.call_once(|| {
        x86_64::install_recursive_mapping();
        x86_64::map_mmio_windows();
        x86_64::map_kernel_heap_window();
    });
}

pub fn current_page_table() -> PageTable {
    x86_64::current()
}

pub fn flush_tlb() {
    x86_64::flush_tlb();
}

pub fn flush_tlb_addr(addr: u64) {
    x86_64::flush_tlb_addr(addr);
}

pub fn switch_to(table: PageTable) {
    x86_64::switch(table);
}
