use crate::arch::memory::VirtAddr;
use crate::arch::x86_64::cpu;
use crate::mm::address_space::AddressSpace;
use crate::mm::cow;
use crate::mm::layout::PAGE_SIZE;
use crate::mm::paging::{PageFlags, PageTable};
use crate::mm::phys;
use crate::mm::permissions::ProtFlags;

#[derive(Clone, Copy, Debug)]
pub struct PageFaultInfo {
    pub address: u64,
    pub error_code: u64,
    pub write: bool,
    pub user: bool,
    pub instruction: bool,
    pub present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFaultOutcome {
    Handled,
    CowBroken,
    DemandPaged,
    ProtectionViolation,
    BadAddress,
}

pub fn decode_error(code: u64) -> PageFaultInfo {
    PageFaultInfo {
        address: cpu::read_cr2(),
        error_code: code,
        present: code & 1 != 0,
        write: code & (1 << 1) != 0,
        user: code & (1 << 2) != 0,
        instruction: code & (1 << 4) != 0,
    }
}

pub fn handle_page_fault(
    space: &mut AddressSpace,
    addr: VirtAddr,
    write: bool,
    user: bool,
) -> bool {
    match resolve_fault(space, addr, write, user) {
        PageFaultOutcome::Handled
        | PageFaultOutcome::CowBroken
        | PageFaultOutcome::DemandPaged => true,
        _ => false,
    }
}

pub fn handle_from_interrupt(error_code: u64) -> PageFaultOutcome {
    let info = decode_error(error_code);
    let addr = VirtAddr::new(info.address);

    if info.user {
        if handle_user_fault(addr, info.write, info.user) {
            return PageFaultOutcome::Handled;
        }
        return PageFaultOutcome::BadAddress;
    }

    // Kernel fault path
    let mut kernel = crate::mm::address_space::KernelAddressSpace::get();
    resolve_fault(&mut kernel, addr, info.write, info.user)
}

fn resolve_fault(
    space: &mut AddressSpace,
    addr: VirtAddr,
    write: bool,
    user: bool,
) -> PageFaultOutcome {
    let page = addr.as_u64() & !(PAGE_SIZE - 1);
    let virt = VirtAddr::new(page);

    let vma = match space.vma.find(page) {
        Some(v) => v,
        None => return PageFaultOutcome::BadAddress,
    };

    if user && !vma.prot.contains(ProtFlags::USER) {
        return PageFaultOutcome::ProtectionViolation;
    }
    if write && !vma.prot.contains(ProtFlags::WRITE) && !cow::is_cow(space.page_table.get_flags(virt).unwrap_or(PageFlags::empty())) {
        return PageFaultOutcome::ProtectionViolation;
    }

    if let Some(flags) = space.page_table.get_flags(virt) {
        if flags.contains(PageFlags::COW) && write {
            if cow::break_cow(space.page_table, virt, flags).is_some() {
                return PageFaultOutcome::CowBroken;
            }
            return PageFaultOutcome::ProtectionViolation;
        }
        if flags.contains(PageFlags::SOFTWARE_DEMAND) || !flags.contains(PageFlags::PRESENT) {
            return demand_page(space, virt, vma.prot);
        }
        if flags.contains(PageFlags::PRESENT) {
            return PageFaultOutcome::Handled;
        }
    }

    demand_page(space, virt, vma.prot)
}

fn demand_page(space: &mut AddressSpace, virt: VirtAddr, prot: ProtFlags) -> PageFaultOutcome {
    let frame = match phys::alloc_frame(space.numa_node) {
        Some(f) => f,
        None => return PageFaultOutcome::ProtectionViolation,
    };
    let hhdm = crate::boot_info().hhdm_offset;
    let dst = crate::arch::memory::phys_to_virt(hhdm, frame.phys());
    unsafe {
        core::ptr::write_bytes(dst.as_mut_ptr::<u8>(), 0, PAGE_SIZE as usize);
    }
    let flags = PageTable::flags_from_prot(prot, true);
    if space.page_table.map_page(virt, frame.phys(), flags).is_err() {
        phys::free_frame(frame);
        return PageFaultOutcome::ProtectionViolation;
    }
    PageFaultOutcome::DemandPaged
}

fn user_space_for_fault() -> Option<crate::mm::address_space::AddressSpace> {
    // Clone path not available; fault handler uses in-place mutation via proc hook.
    None
}

pub fn handle_user_fault(addr: VirtAddr, write: bool, user: bool) -> bool {
    if let Some(result) = crate::proc::with_current_user_address_space(|space| {
        handle_page_fault(space, addr, write, user)
    }) {
        return result;
    }
    false
}
