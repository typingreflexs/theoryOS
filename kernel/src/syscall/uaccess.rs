//! User/kernel pointer validation and safe copying.

use crate::arch::x86_64::features;
use crate::mm::layout::{is_user_address, USER_ADDR_MAX, USER_ADDR_MIN};
use crate::security::kpti;
use crate::syscall::errno::{err, ok, Errno, SysResult};

#[inline]
fn stac() {
    unsafe {
        core::arch::asm!("stac", options(nomem, nostack));
    }
}

#[inline]
fn clac() {
    unsafe {
        core::arch::asm!("clac", options(nomem, nostack));
    }
}

/// Validate that `[addr, addr+len)` lies entirely in canonical userspace.
pub fn access_ok(addr: u64, len: u64) -> bool {
    if len == 0 {
        return addr == 0 || is_user_address(addr);
    }
    if addr == 0 || addr < USER_ADDR_MIN {
        return false;
    }
    if !is_user_address(addr) {
        return false;
    }
    let end = match addr.checked_add(len.saturating_sub(1)) {
        Some(v) => v,
        None => return false,
    };
    is_user_address(end) && end <= USER_ADDR_MAX
}

fn copy_with_smap<F>(f: F) -> SysResult
where
    F: FnOnce() -> SysResult,
{
    kpti::with_user_as(|| {
        if features::smap_enabled() {
            stac();
        }
        let r = f();
        if features::smap_enabled() {
            clac();
        }
        r
    })
}

pub fn copy_from_user(dst: &mut [u8], user_src: u64) -> SysResult {
    if !access_ok(user_src, dst.len() as u64) {
        return err(Errno::EFAULT);
    }
    copy_with_smap(|| {
        unsafe {
            core::ptr::copy_nonoverlapping(user_src as *const u8, dst.as_mut_ptr(), dst.len());
        }
        ok(dst.len() as isize)
    })
}

pub fn copy_to_user(user_dst: u64, src: &[u8]) -> SysResult {
    if !access_ok(user_dst, src.len() as u64) {
        return err(Errno::EFAULT);
    }
    copy_with_smap(|| {
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), user_dst as *mut u8, src.len());
        }
        ok(src.len() as isize)
    })
}

pub fn copy_from_user_obj<T: Copy>(user_src: u64) -> Result<T, Errno> {
    let mut val = core::mem::MaybeUninit::<T>::uninit();
    let size = core::mem::size_of::<T>();
    if !access_ok(user_src, size as u64) {
        return Err(Errno::EFAULT);
    }
    kpti::with_user_as(|| {
        if features::smap_enabled() {
            stac();
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                user_src as *const u8,
                val.as_mut_ptr() as *mut u8,
                size,
            );
        }
        if features::smap_enabled() {
            clac();
        }
    });
    unsafe { Ok(val.assume_init()) }
}

pub fn copy_to_user_obj<T: Copy>(user_dst: u64, val: &T) -> Result<(), Errno> {
    let size = core::mem::size_of::<T>();
    if !access_ok(user_dst, size as u64) {
        return Err(Errno::EFAULT);
    }
    kpti::with_user_as(|| {
        if features::smap_enabled() {
            stac();
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                val as *const T as *const u8,
                user_dst as *mut u8,
                size,
            );
        }
        if features::smap_enabled() {
            clac();
        }
    });
    Ok(())
}

/// Read a NUL-terminated string from userspace (max `max_len` bytes).
pub fn str_from_user(buf: &mut [u8], user_src: u64, max_len: usize) -> Result<usize, Errno> {
    if max_len == 0 || buf.is_empty() {
        return Err(Errno::EINVAL);
    }
    if !access_ok(user_src, 1) {
        return Err(Errno::EFAULT);
    }
    let limit = max_len.min(buf.len());
    kpti::with_user_as(|| {
        if features::smap_enabled() {
            stac();
        }
        for i in 0..limit {
            if !access_ok(user_src + i as u64, 1) {
                if features::smap_enabled() {
                    clac();
                }
                return Err(Errno::EFAULT);
            }
            let byte = unsafe { (user_src as *const u8).add(i).read() };
            buf[i] = byte;
            if byte == 0 {
                if features::smap_enabled() {
                    clac();
                }
                return Ok(i);
            }
        }
        if features::smap_enabled() {
            clac();
        }
        Err(Errno::EINVAL)
    })
}

pub fn user_slice_ok(addr: u64, len: u64) -> Result<(), Errno> {
    if access_ok(addr, len) {
        Ok(())
    } else {
        Err(Errno::EFAULT)
    }
}
