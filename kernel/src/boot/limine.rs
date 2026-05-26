use crate::acpi::RsdpLocation;
use crate::arch::memory::PhysAddr;
use crate::boot::info::{MemoryKind, MemoryRegion};

/// Limine request/response protocol (Limine 12.x, base revision 4).
pub mod requests {
    pub const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

    pub const fn request_id(part2: u64, part3: u64) -> [u64; 4] {
        [
            LIMINE_COMMON_MAGIC[0],
            LIMINE_COMMON_MAGIC[1],
            part2,
            part3,
        ]
    }

    pub const HHDM_REQUEST_ID: [u64; 4] =
        request_id(0x48dcf1cb8ad2b852, 0x63984e959a98244b);
    pub const MEMMAP_REQUEST_ID: [u64; 4] =
        request_id(0x67cf3d9d378a806f, 0xe304acdfc50c3c62);
    pub const RSDP_REQUEST_ID: [u64; 4] =
        request_id(0xc5e77b6b397e7b43, 0x27637845accdcf3c);
    pub const EXECUTABLE_ADDRESS_REQUEST_ID: [u64; 4] =
        request_id(0x71ba76863cc55f63, 0xb2644a48c516a487);
    pub const SMP_REQUEST_ID: [u64; 4] =
        request_id(0x95a67b819a1b857e, 0xa0b61b723b6a73e0);
    pub const COMMAND_LINE_REQUEST_ID: [u64; 4] =
        request_id(0x4b161536e598651e, 0xb390ad4a2f1f303a);
    pub const FRAMEBUFFER_REQUEST_ID: [u64; 4] =
        request_id(0x9d5827dcd881dd75, 0xa3148604f6fab11b);

    pub const LIMINE_BASE_REVISION: u64 = 0;

    #[repr(C)]
    pub struct LimineHhdmRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineHhdmResponse,
    }

    #[repr(C)]
    pub struct LimineHhdmResponse {
        pub revision: u64,
        pub offset: u64,
    }

    #[repr(C)]
    pub struct LimineMemmapRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineMemmapResponse,
    }

    #[repr(C)]
    pub struct LimineMemmapResponse {
        pub revision: u64,
        pub entry_count: u64,
        pub entries: *mut *mut LimineMemmapEntry,
    }

    #[repr(C)]
    pub struct LimineMemmapEntry {
        pub base: u64,
        pub length: u64,
        pub kind: u64,
    }

    #[repr(C)]
    pub struct LimineRsdpRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineRsdpResponse,
    }

    #[repr(C)]
    pub struct LimineRsdpResponse {
        pub revision: u64,
        pub address: u64,
    }

    #[repr(C)]
    pub struct LimineExecutableAddressRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineExecutableAddressResponse,
    }

    #[repr(C)]
    pub struct LimineExecutableAddressResponse {
        pub revision: u64,
        pub physical_base: u64,
        pub virtual_base: u64,
    }

    #[repr(C)]
    pub struct LimineSmpRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineSmpResponse,
        pub flags: u64,
    }

    #[repr(C)]
    pub struct LimineSmpResponse {
        pub revision: u64,
        pub flags: u32,
        pub bsp_lapic_id: u32,
        pub cpu_count: u64,
        pub cpus: *mut *mut LimineSmpInfo,
    }

    #[repr(C)]
    pub struct LimineSmpInfo {
        pub processor_id: u32,
        pub lapic_id: u32,
        pub reserved: u64,
        pub goto_address: u64,
        pub extra_argument: u64,
    }

    #[repr(C)]
    pub struct LimineCommandLineRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineCommandLineResponse,
    }

    #[repr(C)]
    pub struct LimineCommandLineResponse {
        pub revision: u64,
        pub cmdline: *const u8,
    }

    #[repr(C)]
    pub struct LimineFramebufferRequest {
        pub id: [u64; 4],
        pub revision: u64,
        pub response: *mut LimineFramebufferResponse,
    }

    #[repr(C)]
    pub struct LimineFramebufferResponse {
        pub revision: u64,
        pub framebuffer_count: u64,
        pub framebuffers: *mut *mut LimineFramebuffer,
    }

    #[repr(C)]
    pub struct LimineFramebuffer {
        pub address: *mut u32,
        pub width: u64,
        pub height: u64,
        pub pitch: u64,
        pub bpp: u16,
        pub memory_model: u8,
        pub red_mask_size: u8,
        pub red_mask_shift: u8,
        pub green_mask_size: u8,
        pub green_mask_shift: u8,
        pub blue_mask_size: u8,
        pub blue_mask_shift: u8,
        pub unused: [u8; 7],
        pub edid_size: u64,
        pub edid: *mut u8,
    }
}

pub struct LimineRequests;

impl LimineRequests {
    pub fn hhdm() -> &'static requests::LimineHhdmRequest {
        unsafe { &HHDM_REQUEST }
    }

    pub fn memmap() -> &'static requests::LimineMemmapRequest {
        unsafe { &MEMMAP_REQUEST }
    }

    pub fn rsdp() -> &'static requests::LimineRsdpRequest {
        unsafe { &RSDP_REQUEST }
    }

    pub fn kernel_address() -> &'static requests::LimineExecutableAddressRequest {
        unsafe { &EXECUTABLE_ADDRESS_REQUEST }
    }

    pub fn smp() -> &'static requests::LimineSmpRequest {
        unsafe { &SMP_REQUEST }
    }

    pub fn command_line() -> &'static requests::LimineCommandLineRequest {
        unsafe { &COMMAND_LINE_REQUEST }
    }

    pub fn framebuffer() -> &'static requests::LimineFramebufferRequest {
        unsafe { &FRAMEBUFFER_REQUEST }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u16,
    pub virt_addr: u64,
    /// Limine memory model: 0 = RGB, 1 = BGR.
    pub memory_model: u8,
}

#[derive(Debug)]
pub struct LimineBootInfo {
    pub hhdm_offset: u64,
    pub kernel_physical_base: u64,
    pub rsdp: Option<RsdpLocation>,
    pub memory_map: &'static [MemoryRegion],
    pub cmdline: Option<&'static str>,
    pub bsp_lapic_id: u32,
    pub framebuffer: Option<FramebufferInfo>,
}

impl LimineBootInfo {
    pub fn from_requests(_requests: &LimineRequests) -> Option<Self> {
        let hhdm_offset = unsafe {
            let resp = LimineRequests::hhdm().response;
            if resp.is_null() {
                return None;
            }
            (*resp).offset
        };

        let kernel_physical_base = unsafe {
            let resp = LimineRequests::kernel_address().response;
            if resp.is_null() {
                return None;
            }
            (*resp).physical_base
        };

        let rsdp = unsafe {
            let resp = LimineRequests::rsdp().response;
            if resp.is_null() {
                None
            } else {
                let addr = (*resp).address;
                Some(RsdpLocation::from_limine(addr))
            }
        };

        let memory_map = parse_memory_map()?;

        let cmdline = unsafe {
            let resp = LimineRequests::command_line().response;
            if resp.is_null() || (*resp).cmdline.is_null() {
                None
            } else {
                Some(cstr_to_str((*resp).cmdline))
            }
        };

        let bsp_lapic_id = unsafe {
            let resp = LimineRequests::smp().response;
            if resp.is_null() {
                0
            } else {
                (*resp).bsp_lapic_id
            }
        };

        let framebuffer = parse_framebuffer();

        Some(Self {
            hhdm_offset,
            kernel_physical_base,
            rsdp,
            memory_map,
            cmdline,
            bsp_lapic_id,
            framebuffer,
        })
    }
}

fn parse_framebuffer() -> Option<FramebufferInfo> {
    unsafe {
        let resp = LimineRequests::framebuffer().response;
        if resp.is_null() || (*resp).framebuffer_count == 0 {
            return None;
        }
        let list = (*resp).framebuffers;
        if list.is_null() {
            return None;
        }
        let fb_ptr = *list;
        if fb_ptr.is_null() {
            return None;
        }
        let fb = &*fb_ptr;
        if fb.address.is_null() || fb.width == 0 || fb.height == 0 {
            return None;
        }
        Some(FramebufferInfo {
            width: fb.width as u32,
            height: fb.height as u32,
            pitch: fb.pitch as u32,
            bpp: fb.bpp,
            virt_addr: fb.address as u64,
            memory_model: fb.memory_model,
        })
    }
}

fn parse_memory_map() -> Option<&'static [MemoryRegion]> {
    static mut MAP: [MemoryRegion; 256] = [MemoryRegion {
        start: PhysAddr::new(0),
        length: 0,
        kind: MemoryKind::Unknown,
    }; 256];
    static mut MAP_LEN: usize = 0;

    unsafe {
        let resp = LimineRequests::memmap().response;
        if resp.is_null() {
            return None;
        }

        let count = (*resp).entry_count as usize;
        if count > MAP.len() {
            return None;
        }

        let entries = core::slice::from_raw_parts((*resp).entries, count);
        for (idx, entry_ptr) in entries.iter().enumerate() {
            if entry_ptr.is_null() {
                return None;
            }
            let entry = &**entry_ptr;
            MAP[idx] = MemoryRegion {
                start: PhysAddr::new(entry.base),
                length: entry.length,
                kind: limine_mem_kind(entry.kind),
            };
        }
        MAP_LEN = count;
        Some(core::slice::from_raw_parts(MAP.as_ptr(), MAP_LEN))
    }
}

fn limine_mem_kind(kind: u64) -> MemoryKind {
    match kind {
        0 => MemoryKind::Usable,
        1 => MemoryKind::Reserved,
        2 => MemoryKind::AcpiReclaimable,
        3 => MemoryKind::AcpiNvs,
        4 => MemoryKind::BadMemory,
        5 => MemoryKind::BootloaderReclaimable,
        _ => MemoryKind::Unknown,
    }
}

unsafe fn cstr_to_str(ptr: *const u8) -> &'static str {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
}

#[used]
#[link_section = ".limine_requests_start"]
static LIMINE_REQUESTS_START_MARKER: [u64; 4] = [
    0xf6b8f4b39de7d1ae,
    0xfab91a6940fcb9cf,
    0x785c6ed015d3e316,
    0x181e920a7852b9d9,
];

#[used]
#[link_section = ".limine_requests"]
static LIMINE_BASE_REVISION_TAG: [u64; 3] = [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, 4];

#[used]
#[link_section = ".limine_requests"]
static mut HHDM_REQUEST: requests::LimineHhdmRequest = requests::LimineHhdmRequest {
    id: requests::HHDM_REQUEST_ID,
    revision: requests::LIMINE_BASE_REVISION,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut MEMMAP_REQUEST: requests::LimineMemmapRequest = requests::LimineMemmapRequest {
    id: requests::MEMMAP_REQUEST_ID,
    revision: requests::LIMINE_BASE_REVISION,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut RSDP_REQUEST: requests::LimineRsdpRequest = requests::LimineRsdpRequest {
    id: requests::RSDP_REQUEST_ID,
    revision: requests::LIMINE_BASE_REVISION,
    response: core::ptr::null_mut(),
};

#[used]
#[link_section = ".limine_requests"]
static mut EXECUTABLE_ADDRESS_REQUEST: requests::LimineExecutableAddressRequest =
    requests::LimineExecutableAddressRequest {
        id: requests::EXECUTABLE_ADDRESS_REQUEST_ID,
        revision: requests::LIMINE_BASE_REVISION,
        response: core::ptr::null_mut(),
    };

#[used]
#[link_section = ".limine_requests"]
static mut SMP_REQUEST: requests::LimineSmpRequest = requests::LimineSmpRequest {
    id: requests::SMP_REQUEST_ID,
    revision: requests::LIMINE_BASE_REVISION,
    response: core::ptr::null_mut(),
    flags: 0,
};

#[used]
#[link_section = ".limine_requests"]
static mut COMMAND_LINE_REQUEST: requests::LimineCommandLineRequest =
    requests::LimineCommandLineRequest {
        id: requests::COMMAND_LINE_REQUEST_ID,
        revision: requests::LIMINE_BASE_REVISION,
        response: core::ptr::null_mut(),
    };

#[used]
#[link_section = ".limine_requests"]
static mut FRAMEBUFFER_REQUEST: requests::LimineFramebufferRequest =
    requests::LimineFramebufferRequest {
        id: requests::FRAMEBUFFER_REQUEST_ID,
        revision: requests::LIMINE_BASE_REVISION,
        response: core::ptr::null_mut(),
    };

#[used]
#[link_section = ".limine_requests_end"]
static LIMINE_REQUESTS_END_MARKER: [u64; 2] = [0xadc0e0531bb10d03, 0x9572709f31764c62];
