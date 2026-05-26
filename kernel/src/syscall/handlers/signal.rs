//! Signal syscalls.

use crate::proc::{self, id::Pid};
use crate::signal::{self, SigAction, SigSet};
use crate::syscall::errno::{err, ok, Errno, SysResult};
use crate::syscall::uaccess::{copy_from_user_obj, copy_to_user_obj, user_slice_ok};

#[repr(C)]
#[derive(Clone, Copy)]
struct UserSigaction {
    handler: u64,
    flags: u64,
    restorer: u64,
    mask: u64,
}

pub fn sys_rt_sigaction(
    sig: u64,
    act_ptr: u64,
    old_ptr: u64,
    _: u64,
    _: u64,
    _: u64,
) -> SysResult {
    if sig == 0 || sig >= crate::signal::NSIG as u64 {
        return err(Errno::EINVAL);
    }
    if act_ptr != 0 {
        user_slice_ok(act_ptr, core::mem::size_of::<UserSigaction>() as u64)?;
        let user_act: UserSigaction = copy_from_user_obj(act_ptr)?;
        let action = SigAction {
            handler: user_act.handler,
            flags: user_act.flags,
            restorer: user_act.restorer,
            mask: SigSet(user_act.mask),
        };
        let old = signal::set_action(sig as u32, action).map_err(|e| Errno(-e))?;
        if old_ptr != 0 {
            write_old_action(old_ptr, old)?;
        }
    } else if old_ptr != 0 {
        let old = proc_old_action(sig as u32)?;
        write_old_action(old_ptr, old)?;
    }
    ok(0)
}

fn proc_old_action(sig: u32) -> Result<SigAction, Errno> {
    crate::proc::current_process_mut(|p| p.signals.actions[sig as usize])
        .ok_or(Errno::EINVAL)
}

fn write_old_action(old_ptr: u64, old: SigAction) -> Result<(), Errno> {
    let user = UserSigaction {
        handler: old.handler,
        flags: old.flags,
        restorer: old.restorer,
        mask: old.mask.0,
    };
    copy_to_user_obj(old_ptr, &user)
}

pub fn sys_rt_sigprocmask(
    how: u64,
    set_ptr: u64,
    old_ptr: u64,
    _: u64,
    _: u64,
    _: u64,
) -> SysResult {
    let mut new_mask = None;
    if set_ptr != 0 {
        user_slice_ok(set_ptr, 8)?;
        let m: u64 = copy_from_user_obj(set_ptr)?;
        new_mask = Some(SigSet(m));
    }
    let old = crate::proc::current_process_mut(|p| {
        let prev = p.signals.blocked;
        if let Some(m) = new_mask {
            match how {
                0 => p.signals.blocked = m,          // SIG_BLOCK
                1 => p.signals.blocked.0 &= !m.0,    // SIG_UNBLOCK
                2 => p.signals.blocked = m,          // SIG_SETMASK
                _ => {}
            }
        }
        prev
    })
    .ok_or(Errno::EINVAL)?;
    if old_ptr != 0 {
        copy_to_user_obj(old_ptr, &old.0)?;
    }
    ok(0)
}

pub fn sys_rt_sigreturn(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    // Handled specially in syscall dispatch with frame access
    err(Errno::ENOSYS)
}

pub fn sys_kill(pid: u64, sig: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let target = crate::proc::id::Pid::new(pid as u32);
    let caller_pid = proc::current_thread(|t| t.pid).unwrap_or(Pid::KERNEL);

    if target != caller_pid {
        let allowed = proc::current_process_mut(|caller| {
            proc::table::with_process(target, |target_proc| caller.cred.can_signal(&target_proc.cred))
        })
        .flatten()
        .unwrap_or(false);
        if !allowed {
            return err(Errno::EPERM);
        }
    }

    match signal::kill(target, sig as u32) {
        Ok(()) => ok(0),
        Err(e) => Err(Errno(-e)),
    }
}

pub fn sys_signal(sig: u64, handler: u64, _: u64, _: u64, _: u64, _: u64) -> SysResult {
    let action = SigAction {
        handler,
        flags: 0,
        restorer: 0,
        mask: SigSet::empty(),
    };
    match signal::set_action(sig as u32, action) {
        Ok(_) => ok(0),
        Err(e) => Err(Errno(-e)),
    }
}
