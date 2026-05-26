use crate::arch::memory::{PhysAddr, VirtAddr};
use crate::boot::limine::FramebufferInfo;
use crate::boot_info;
use crate::mm::layout::{align_up, PAGE_SIZE, USER_FB_BASE, USER_FB_META_BASE};

/// Magic value in `UserFbMeta::magic` — ASCII "FB0\0".
pub const FB_MAGIC: u32 = 0x3042_4646;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Bgr,
}

#[derive(Clone, Copy, Debug)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u16,
    pub virt_addr: VirtAddr,
    pub byte_size: u64,
    pub format: PixelFormat,
    /// Map logical X (left-to-right) to framebuffer memory X.
    pub flip_x: bool,
    /// Map logical Y (top-to-bottom) to framebuffer memory Y.
    pub flip_y: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserFbMeta {
    pub magic: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub pixels: u64,
}

impl Framebuffer {
    pub fn from_boot() -> Option<Self> {
        let info = boot_info().framebuffer?;
        Some(Self::from_info(info))
    }

    pub fn from_info(info: FramebufferInfo) -> Self {
        let byte_size = align_up(info.pitch as u64 * info.height as u64, PAGE_SIZE);
        let format = if info.memory_model == 0 {
            PixelFormat::Rgb
        } else {
            PixelFormat::Bgr
        };
        Self {
            width: info.width,
            height: info.height,
            pitch: info.pitch,
            bpp: info.bpp,
            virt_addr: VirtAddr::new(info.virt_addr),
            byte_size,
            format,
            // Limine maps the FB with y=0 at the top-left; no axis swap needed.
            flip_x: false,
            flip_y: false,
        }
    }

    pub fn pixels(&self) -> *mut u32 {
        self.virt_addr.as_mut_ptr()
    }

    pub fn phys_base(&self) -> Option<PhysAddr> {
        boot_info().virt_to_phys(self.virt_addr.as_u64())
    }

    pub fn page_count(&self) -> u64 {
        self.byte_size / PAGE_SIZE
    }

    pub fn pack_color(&self, r: u8, g: u8, b: u8) -> u32 {
        match self.format {
            PixelFormat::Rgb => u32::from_le_bytes([r, g, b, 0xFF]),
            PixelFormat::Bgr => u32::from_le_bytes([b, g, r, 0xFF]),
        }
    }

    pub fn user_meta(&self) -> UserFbMeta {
        UserFbMeta {
            magic: FB_MAGIC,
            width: self.width,
            height: self.height,
            pitch: self.pitch,
            bpp: self.bpp as u32,
            pixels: USER_FB_BASE,
        }
    }

    pub fn user_meta_addr(&self) -> u64 {
        USER_FB_META_BASE
    }
}
