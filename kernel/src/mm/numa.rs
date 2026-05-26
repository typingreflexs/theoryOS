use crate::arch::memory::PhysAddr;
use crate::boot::info::MemoryRegion;
use crate::mm::layout::PAGE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NumaNodeId(u32);

impl NumaNodeId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumaPolicy {
    Local,
    Preferred(NumaNodeId),
    Interleave,
    Bind(NumaNodeId),
}

#[derive(Clone, Copy, Debug)]
struct NumaRange {
    start: PhysAddr,
    end: PhysAddr,
    node: NumaNodeId,
}

const MAX_RANGES: usize = 64;

static mut NODE_COUNT: u32 = 1;
static mut RANGES: [Option<NumaRange>; MAX_RANGES] = [None; MAX_RANGES];
static mut RANGE_LEN: usize = 0;

pub fn init() {
    if let Some(ctx) = crate::acpi::context() {
        if let Some(srat) = ctx.tables().srat() {
            install_srat(srat);
            return;
        }
    }
    install_from_memory_map(crate::boot_info().memory_map);
}

pub fn node_count() -> u32 {
    unsafe { NODE_COUNT }
}

pub fn local_node() -> NumaNodeId {
    NumaNodeId::new(0)
}

pub fn node_for_phys(addr: PhysAddr) -> NumaNodeId {
    unsafe {
        for entry in RANGES.iter().take(RANGE_LEN).flatten() {
            if addr.as_u64() >= entry.start.as_u64() && addr.as_u64() < entry.end.as_u64() {
                return entry.node;
            }
        }
    }
    local_node()
}

pub fn choose_node(policy: NumaPolicy) -> NumaNodeId {
    match policy {
        NumaPolicy::Local => local_node(),
        NumaPolicy::Preferred(node) | NumaPolicy::Bind(node) => node,
        NumaPolicy::Interleave => {
            static mut TICK: u32 = 0;
            unsafe {
                TICK = TICK.wrapping_add(1);
                NumaNodeId::new(TICK % NODE_COUNT.max(1))
            }
        }
    }
}

fn install_from_memory_map(map: &[MemoryRegion]) {
    unsafe {
        NODE_COUNT = 1;
        RANGE_LEN = 0;
        for region in map {
            if RANGE_LEN >= MAX_RANGES {
                break;
            }
            RANGES[RANGE_LEN] = Some(NumaRange {
                start: region.start,
                end: PhysAddr::new(region.start.as_u64() + region.length),
                node: NumaNodeId::new(0),
            });
            RANGE_LEN += 1;
        }
    }
}

fn install_srat(srat: &crate::acpi::srat::Srat) {
    unsafe {
        NODE_COUNT = srat.node_count().max(1);
        RANGE_LEN = 0;
        for affinity in srat.memory_affinities() {
            if RANGE_LEN >= MAX_RANGES {
                break;
            }
            RANGES[RANGE_LEN] = Some(NumaRange {
                start: affinity.base,
                end: PhysAddr::new(affinity.base.as_u64() + affinity.length),
                node: NumaNodeId::new(affinity.node as u32),
            });
            RANGE_LEN += 1;
        }
    }
}
