use spin::{Mutex, Once};

use crate::arch::memory::VirtAddr;
use crate::mm::aslr;
use crate::mm::cow;
use crate::mm::layout::{align_up, PAGE_SIZE};
use crate::mm::numa::NumaNodeId;
use crate::mm::paging::{PageTable, PhysFrame};
use crate::mm::permissions::ProtFlags;
use crate::mm::vma::VmaTree;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddressSpaceId(u64);

impl AddressSpaceId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
pub struct AddressSpace {
    pub id: AddressSpaceId,
    pub page_table: PageTable,
    pub vma: VmaTree,
    pub numa_node: NumaNodeId,
}

static NEXT_ID: Mutex<u64> = Mutex::new(1);
static KERNEL_SPACE: Once<Mutex<AddressSpace>> = Once::new();
static CURRENT: Mutex<Option<AddressSpaceId>> = Mutex::new(None);

pub struct KernelAddressSpace;

pub fn init() {
    let mut space = AddressSpace::new_kernel().expect("kernel address space");
    let heap_base = aslr::kernel_heap_base();
    let _ = space.vma.insert(crate::mm::vma::Vma::new(
        heap_base,
        crate::mm::layout::KERNEL_HEAP_SIZE,
        ProtFlags::READ | ProtFlags::WRITE,
        crate::mm::permissions::MmapFlags::ANONYMOUS | crate::mm::permissions::MmapFlags::PRIVATE,
        crate::mm::vma::VmaKind::Heap,
    ));
    let _ = KERNEL_SPACE.call_once(|| Mutex::new(space));
}

impl AddressSpace {
    pub fn new_user() -> Option<Self> {
        let id = AddressSpaceId::new({
            let mut next = NEXT_ID.lock();
            let id = *next;
            *next += 1;
            id
        });
        let page_table = PageTable::new_empty()?;
        crate::security::kpti::init_user_page_table(&page_table);
        Some(Self {
            id,
            page_table,
            vma: VmaTree::new(),
            numa_node: crate::mm::numa::local_node(),
        })
    }

    pub fn new_kernel() -> Option<Self> {
        Some(Self {
            id: AddressSpaceId::new(0),
            page_table: PageTable::kernel(),
            vma: VmaTree::new(),
            numa_node: crate::mm::numa::local_node(),
        })
    }

    pub fn activate(&self) {
        crate::mm::paging::switch_to(self.page_table);
        *CURRENT.lock() = Some(self.id);
    }

    pub fn fork_from(parent: &Self) -> Option<Self> {
        let mut child = Self::new_user()?;
        child.numa_node = parent.numa_node;
        for vma in parent.vma.iter() {
            child.vma.insert(*vma).ok()?;
            cow::duplicate_vma(parent.page_table, child.page_table, vma)?;
        }
        Some(child)
    }

    pub fn handle_fault(&mut self, addr: VirtAddr, write: bool, user: bool) -> bool {
        crate::mm::fault::handle_page_fault(self, addr, write, user)
    }
}

impl KernelAddressSpace {
    pub fn get() -> spin::MutexGuard<'static, AddressSpace> {
        KERNEL_SPACE.get().expect("kernel AS not ready").lock()
    }
}

pub fn current_id() -> Option<AddressSpaceId> {
    *CURRENT.lock()
}

pub fn with_current<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut AddressSpace) -> R,
{
    let id = current_id()?;
    if id.as_u64() == 0 {
        let mut k = KernelAddressSpace::get();
        return Some(f(&mut k));
    }
    None
}

pub fn map_anonymous(
    space: &mut AddressSpace,
    start: u64,
    length: u64,
    prot: ProtFlags,
) -> Result<(), ()> {
    let length = align_up(length, PAGE_SIZE);
    crate::mm::vm::mmap(
        space,
        start,
        length,
        prot,
        crate::mm::permissions::MmapFlags::ANONYMOUS
            | crate::mm::permissions::MmapFlags::PRIVATE,
        0,
    )
    .map(|_| ())
    .map_err(|_| ())
}
