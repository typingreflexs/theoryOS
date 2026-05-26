//! Memory management — physical frames, paging, heap, VMAs, COW, ELF loading.
//!
//! Initialization order: phys → paging → numa → aslr → address spaces →
//! IDT (page faults) → slab heap → vm manager.

pub mod address_space;
pub mod aslr;
pub mod cow;
pub mod elf;
pub mod fault;
pub mod heap;
pub mod layout;
pub mod numa;
pub mod paging;
pub mod permissions;
pub mod phys;
pub mod vm;
pub mod vma;

use crate::console::Console;

pub use address_space::{AddressSpace, AddressSpaceId, KernelAddressSpace};
pub use fault::{PageFaultInfo, PageFaultOutcome};
pub use heap::KernelAllocator;
pub use layout::{PageSize, PAGE_SIZE};
pub use numa::{NumaNodeId, NumaPolicy};
pub use paging::{PageFlags, PageTable, PhysFrame};
pub use permissions::{MmapFlags, MprotectFlags, ProtFlags};
pub use phys::{FrameAllocator, FrameDescriptor, PhysicalMemoryStats};
pub use vm::{VmError, VmManager};
pub use vma::{Vma, VmaKind, VmaTree};

/// Initialize the full memory management stack.
pub fn init() {
    Console::println("[mm] phys init");
    phys::init();
    Console::println("[mm] paging init");
    paging::init();
    Console::println("[mm] numa init");
    numa::init();
    aslr::init();
    address_space::init();
    crate::arch::interrupt_init();
    Console::println("[mm] heap init");
    heap::init();
    vm::init();

    let stats = phys::stats();
    Console::println("[mm] physical memory online");
    let _ = core::fmt::Write::write_fmt(
        &mut Console,
        format_args!(
            "[mm] frames: {} usable / {} total ({} MiB free)\n",
            stats.free_frames,
            stats.total_frames,
            stats.free_bytes / (1024 * 1024)
        ),
    );
    Console::println("[mm] paging, slab heap, VMA/VM, COW, NUMA ready");
}
