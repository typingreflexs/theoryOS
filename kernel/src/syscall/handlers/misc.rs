//! Miscellaneous syscalls: uname, arch_prctl, exit_group, set_tid_address, getuid.

use crate::arch::x86_64::cpu;
use crate::proc;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::uaccess::{copy_to_user_obj, user_slice_ok};

#[repr(C)]
#[derive(Clone, Copy)]
struct Utsname {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

const ARCH_SET_FS: u32 = 0x1002;
const ARCH_GET_FS: u32 = 0x1003;

/// IA32_FS_BASE MSR — per Intel SDM vol.4; must be updated on every TLS change for current thread.
const MSR_FS_BASE: u32 = 0xC000_0100;

pub fn sys_uname(buf: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    user_slice_ok(buf, core::mem::size_of::<Utsname>() as u64)?;
    let mut uts = Utsname {
        sysname: [0; 65],
        nodename: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
        domainname: [0; 65],
    };
    write_field(&mut uts.sysname, b"Linux");
    write_field(&mut uts.nodename, b"theory");
    write_field(&mut uts.release, b"6.1.0-theory");
    write_field(&mut uts.version, b"#1 Theory OS");
    write_field(&mut uts.machine, b"x86_64");
    write_field(&mut uts.domainname, b"local");
    copy_to_user_obj(buf, &uts)?;
    ok(0)
}

fn write_field(dst: &mut [u8; 65], src: &[u8]) {
    let n = src.len().min(64);
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn sys_arch_prctl(code: u64, addr: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    match code as u32 {
        ARCH_SET_FS => {
            proc::current_thread_mut(|t| {
                t.fs_base = addr;
                // Program FS base for running thread; restored on context switch for others.
                cpu::write_msr(MSR_FS_BASE, addr);
            })
            .ok_or(Errno::ESRCH)?;
            ok(0)
        }
        ARCH_GET_FS => {
            let base = proc::current_thread(|t| t.fs_base).unwrap_or(0);
            user_slice_ok(addr, 8)?;
            copy_to_user_obj(addr, &base)?;
            ok(0)
        }
        _ => err(Errno::EINVAL),
    }
}

pub fn sys_set_tid_address(uaddr: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    proc::current_thread_mut(|t| {
        t.clear_child_tid = uaddr;
    })
    .ok_or(Errno::ESRCH)?;
    ok(proc::current_tid().as_u32() as isize)
}

pub fn sys_exit_group(status: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    crate::syscall::handlers::proc::sys_exit(status, 0, 0, 0, 0, 0)
}

pub fn sys_getuid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    ok(proc::current_process_mut(|p| p.cred.uid as isize)
        .unwrap_or(0))
}

pub fn sys_geteuid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    ok(proc::current_process_mut(|p| p.cred.euid as isize)
        .unwrap_or(0))
}

pub fn sys_getgid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    ok(proc::current_process_mut(|p| p.cred.gid as isize)
        .unwrap_or(0))
}

pub fn sys_getegid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    ok(proc::current_process_mut(|p| p.cred.egid as isize)
        .unwrap_or(0))
}

pub fn sys_nanosleep(req_ptr: u64, rem_ptr: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    user_slice_ok(req_ptr, 16)?;
    let req: Timespec = crate::syscall::uaccess::copy_from_user_obj(req_ptr)?;
    if req.tv_sec < 0 || req.tv_nsec < 0 || req.tv_nsec >= 1_000_000_000 {
        return err(Errno::EINVAL);
    }
    let deadline = crate::sched::timer::monotonic_ns()
        .saturating_add(req.tv_sec as u64 * 1_000_000_000 + req.tv_nsec as u64);
    while crate::sched::timer::monotonic_ns() < deadline {
        crate::sched::timer::enable_preemption();
        crate::arch::x86_64::cpu::pause();
        crate::sched::timer::disable_preemption();
    }
    if rem_ptr != 0 {
        user_slice_ok(rem_ptr, 16)?;
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        copy_to_user_obj(rem_ptr, &zero)?;
    }
    ok(0)
}

pub fn sys_clock_gettime(clock_id: u64, buf: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let _ = clock_id;
    user_slice_ok(buf, 16)?;
    let ts = Timespec {
        tv_sec: crate::sched::timer::monotonic_ns() as i64 / 1_000_000_000,
        tv_nsec: (crate::sched::timer::monotonic_ns() % 1_000_000_000) as i64,
    };
    copy_to_user_obj(buf, &ts)?;
    ok(0)
}
