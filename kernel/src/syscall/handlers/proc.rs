//! Process-related syscalls: fork, clone, exec, exit, wait, getpid.

use alloc::vec::Vec;

use crate::mm::address_space::AddressSpace;
use crate::proc::{self, id::Pid, ProcessState};
use crate::sched::runqueue;
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::uaccess::{copy_to_user_obj, str_from_user, user_slice_ok};

pub fn sys_getpid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let pid = proc::current_thread(|t| t.pid).unwrap_or(Pid::KERNEL);
    ok(pid.as_u32() as isize)
}

pub fn sys_getppid(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let tid = proc::current_tid();
    let pid = proc::table::with_thread(tid, |t| t.pid).unwrap_or(Pid::KERNEL);
    let ppid = proc::table::with_process(pid, |p| p.parent).unwrap_or(Pid::KERNEL);
    ok(ppid.as_u32() as isize)
}

pub fn sys_fork(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    do_fork(false)
}

/// Minimal clone(2) — supports CLONE_VM-less fork-like behavior and thread flags ignored.
pub fn sys_clone(_flags: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    do_fork(false)
}

fn do_fork(_share_vm: bool) -> SysResult {
    let parent_tid = proc::current_tid();
    let parent_pid = proc::table::with_thread(parent_tid, |t| t.pid).ok_or(Errno::ESRCH)?;

    let (child_as, parent_brk, parent_cwd) = proc::table::with_process(parent_pid, |p| {
        let as_ = p
            .address_space
            .as_ref()
            .and_then(|as_| AddressSpace::fork_from(as_));
        as_.map(|as_| (as_, p.brk, p.cwd))
    })
    .flatten()
    .ok_or(Errno::ENOMEM)?;

    let child_pid = proc::alloc_pid();
    let child_tid = proc::alloc_tid();

    let parent_ctx = proc::table::with_thread(parent_tid, |t| t.context).ok_or(Errno::ESRCH)?;

    let mut child = proc::thread::Thread::new_user(child_tid, child_pid, parent_ctx.rip, parent_ctx.rsp);
    child.context = parent_ctx;
    child.context.rax = 0;
    child.fs_base = proc::table::with_thread(parent_tid, |t| t.fs_base).unwrap_or(0);

    let parent_cred = proc::table::with_process(parent_pid, |p| p.cred).ok_or(Errno::ESRCH)?;
    let parent_seccomp = proc::table::with_process(parent_pid, |p| p.seccomp).ok_or(Errno::ESRCH)?;
    let parent_signals = proc::table::with_process(parent_pid, |p| p.signals).ok_or(Errno::ESRCH)?;

    let mut child_proc = crate::proc::pcb::Process::new_user(child_pid, parent_pid, child_as);
    child_proc.cred = parent_cred.fork();
    child_proc.seccomp = parent_seccomp;
    child_proc.signals = crate::signal::fork_state(&parent_signals);
    child_proc.brk = parent_brk;
    child_proc.cwd = parent_cwd;
    child_proc.add_thread(child_tid);
    child_proc.state = ProcessState::Running;

    proc::table::insert_process(child_proc).ok_or(Errno::ENOMEM)?;
    proc::table::insert_thread(child).ok_or(Errno::ENOMEM)?;

    proc::table::with_process_mut(parent_pid, |p| {
        p.add_child(child_pid);
    });

    let cpu = crate::arch::x86_64::smp::current_cpu_index();
    runqueue::enqueue_thread(child_tid, cpu);

    ok(child_pid.as_u32() as isize)
}

pub fn sys_exit(status: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let tid = proc::current_tid();
    let pid = proc::table::with_thread(tid, |t| t.pid).ok_or(Errno::ESRCH)?;
    let parent = proc::table::with_process(pid, |p| p.parent).ok_or(Errno::ESRCH)?;

    // set_tid_address: futex wake on thread exit (glibc/musl join support).
    let clear_tid = proc::table::with_thread(tid, |t| t.clear_child_tid).unwrap_or(0);
    if clear_tid != 0 {
        let _ = crate::ipc::futex::wake_user(clear_tid, 1);
    }

    proc::table::with_process_mut(pid, |p| {
        p.state = ProcessState::Zombie;
        p.exit_code = status as i32;
    });

    proc::table::with_thread_mut(tid, |t| {
        t.state = crate::proc::thread::ThreadState::Zombie;
    });

    // Wake parent blocked in wait4.
    proc::table::with_process_mut(parent, |p| {
        p.child_wait.wake_one();
    });

    crate::sched::block_current();
    ok(0)
}

pub fn sys_wait4(pid_arg: u64, status_ptr: u64, options: u64, _: u64, _: u64, _: u64) -> SysResult {
    let _ = options;
    let parent_tid = proc::current_tid();
    let parent_pid = proc::table::with_thread(parent_tid, |t| t.pid).ok_or(Errno::ESRCH)?;

    loop {
        if let Some((child, code)) = find_zombie_child(parent_pid, pid_arg as i32) {
            if status_ptr != 0 {
                user_slice_ok(status_ptr, 4)?;
                copy_to_user_obj(status_ptr, &(code as i32))?;
            }
            proc::table::with_process_mut(child, |p| {
                p.state = ProcessState::Dead;
            });
            return ok(child.as_u32() as isize);
        }
        let has_children = proc::table::with_process(parent_pid, |p| p.child_count > 0).unwrap_or(false);
        if !has_children {
            return err(Errno::ECHILD);
        }
        // Block until child exit wakes us via child_wait.
        proc::table::with_process_mut(parent_pid, |p| {
            p.child_wait.block_on();
        });
    }
}

fn find_zombie_child(parent: Pid, wait_pid: i32) -> Option<(Pid, i32)> {
    let children = proc::table::with_process(parent, |p| {
        let mut out = [(Pid::INVALID, 0i32); MAX_CHILDREN];
        let mut n = 0usize;
        for i in 0..p.child_count {
            out[n] = (p.children[i], 0);
            n += 1;
        }
        (out, n)
    })?;

    for i in 0..children.1 {
        let child = children.0[i].0;
        if !child.is_valid() {
            continue;
        }
        if wait_pid > 0 && child.as_u32() as i32 != wait_pid {
            continue;
        }
        if wait_pid != -1 && wait_pid <= 0 {
            // waitpid(-1) or waitpid(0) — accept any child; 0 means any in same group (simplified).
        }
        match proc::table::with_process(child, |c| {
            if c.state == ProcessState::Zombie {
                Some(c.exit_code)
            } else {
                None
            }
        }) {
            Some(Some(code)) => return Some((child, code)),
            _ => {}
        }
    }
    None
}

const MAX_CHILDREN: usize = crate::proc::pcb::MAX_CHILDREN;

pub fn sys_execve(path_ptr: u64, argv_ptr: u64, envp_ptr: u64, _: u64, _: u64, _: u64) -> SysResult {
    let mut path = [0u8; 256];
    str_from_user(&mut path, path_ptr, 255)?;
    let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
    let path = &path[..end];

    let argv = parse_user_argv(argv_ptr)?;
    let envp = parse_user_envp(envp_ptr)?;

    let tid = proc::current_tid();
    let pid = proc::table::with_thread(tid, |t| t.pid).ok_or(Errno::ESRCH)?;

    let argv_refs: Vec<&[u8]> = argv.iter().map(|v| v.as_slice()).collect();
    let envp_refs: Vec<&[u8]> = envp.iter().map(|v| v.as_slice()).collect();

    crate::proc::exec::kernel_exec(pid, tid, path, &argv_refs, &envp_refs)
        .map_err(crate::proc::exec::exec_err_to_errno)?;
    ok(0)
}

fn parse_user_argv(ptr: u64) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, Errno> {
    let mut out = alloc::vec::Vec::new();
    if ptr == 0 {
        out.push(b"init".to_vec());
        return Ok(out);
    }
    let mut i = 0u64;
    loop {
        user_slice_ok(ptr + i * 8, 8)?;
        let s_ptr: u64 = copy_from_user_obj(ptr + i * 8)?;
        if s_ptr == 0 {
            break;
        }
        let mut s = [0u8; 256];
        str_from_user(&mut s, s_ptr, 255)?;
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        out.push(s[..end].to_vec());
        i += 1;
        if i > 256 {
            break;
        }
    }
    if out.is_empty() {
        out.push(b"a.out".to_vec());
    }
    Ok(out)
}

fn parse_user_envp(ptr: u64) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, Errno> {
    let mut out = alloc::vec::Vec::new();
    if ptr == 0 {
        return Ok(out);
    }
    let mut i = 0u64;
    loop {
        user_slice_ok(ptr + i * 8, 8)?;
        let s_ptr: u64 = copy_from_user_obj(ptr + i * 8)?;
        if s_ptr == 0 {
            break;
        }
        let mut s = [0u8; 256];
        str_from_user(&mut s, s_ptr, 255)?;
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        out.push(s[..end].to_vec());
        i += 1;
        if i > 256 {
            break;
        }
    }
    Ok(out)
}

fn copy_from_user_obj<T: Copy>(addr: u64) -> Result<T, Errno> {
    crate::syscall::uaccess::copy_from_user_obj(addr)
}
