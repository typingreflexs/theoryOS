use core::arch::asm;

use crate::arch::x86_64::gdt::sel;
use crate::arch::x86_64::interrupts::InterruptFrame;
use crate::arch::x86_64::xsave::{self, ExtendedState};

/// Saved CPU register context (matches interrupt stub layout minus vector/error).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CpuContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl CpuContext {
    pub const fn empty() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: 0,
            rsi: 0,
            rbp: 0,
            rbx: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,
            rip: 0,
            cs: sel::KERNEL_CODE as u64,
            rflags: 0x202,
            rsp: 0,
            ss: sel::KERNEL_DATA as u64,
        }
    }

    pub fn from_interrupt(frame: &InterruptFrame) -> Self {
        Self {
            r15: frame.r15,
            r14: frame.r14,
            r13: frame.r13,
            r12: frame.r12,
            r11: frame.r11,
            r10: frame.r10,
            r9: frame.r9,
            r8: frame.r8,
            rdi: frame.rdi,
            rsi: frame.rsi,
            rbp: frame.rbp,
            rbx: frame.rbx,
            rdx: frame.rdx,
            rcx: frame.rcx,
            rax: frame.rax,
            rip: frame.rip,
            cs: frame.cs,
            rflags: frame.rflags,
            rsp: frame.rsp,
            ss: frame.ss,
        }
    }

    pub fn write_interrupt(&self, frame: &mut InterruptFrame) {
        frame.r15 = self.r15;
        frame.r14 = self.r14;
        frame.r13 = self.r13;
        frame.r12 = self.r12;
        frame.r11 = self.r11;
        frame.r10 = self.r10;
        frame.r9 = self.r9;
        frame.r8 = self.r8;
        frame.rdi = self.rdi;
        frame.rsi = self.rsi;
        frame.rbp = self.rbp;
        frame.rbx = self.rbx;
        frame.rdx = self.rdx;
        frame.rcx = self.rcx;
        frame.rax = self.rax;
        frame.rip = self.rip;
        frame.cs = self.cs;
        frame.rflags = self.rflags;
        frame.rsp = self.rsp;
        frame.ss = self.ss;
    }

    /// Bootstrap a kernel thread on its stack (fake interrupt return frame).
    pub fn init_kernel_thread(stack_top: u64, entry: extern "C" fn() -> !) -> Self {
        let mut ctx = Self::empty();
        ctx.rsp = stack_top;
        ctx.rip = entry as u64;
        ctx.cs = sel::KERNEL_CODE as u64;
        ctx.ss = sel::KERNEL_DATA as u64;
        ctx.rflags = 0x202;
        ctx
    }

    /// Bootstrap a user thread (ring 3 entry via iretq).
    pub fn init_user_thread(user_stack: u64, entry: u64) -> Self {
        let mut ctx = Self::empty();
        ctx.rsp = user_stack;
        ctx.rip = entry;
        ctx.cs = sel::USER_CODE as u64;
        ctx.ss = sel::USER_DATA as u64;
        ctx.rflags = 0x202;
        ctx
    }
}

/// Save extended FPU/SSE/AVX state and GPR context from an interrupt frame.
pub fn save_thread_context(ctx: &mut CpuContext, ext: &mut ExtendedState, frame: &InterruptFrame) {
    *ctx = CpuContext::from_interrupt(frame);
    xsave::save(ext);
}

/// Restore extended state and overwrite the interrupt frame for iretq return.
pub fn restore_thread_context(ctx: &CpuContext, ext: &ExtendedState, frame: &mut InterruptFrame) {
    xsave::restore(ext);
    ctx.write_interrupt(frame);
}

#[cfg(not(test))]
extern "C" {
    fn theory_context_switch(old: *mut CpuContext, new: *const CpuContext);
}

/// Cooperative context switch (kernel threads, not from interrupt).
#[cfg(not(test))]
pub fn switch_context(old: &mut CpuContext, new: &CpuContext) {
    // SAFETY: assembly stub saves/restores register frame at valid CpuContext pointers.
    unsafe {
        theory_context_switch(old, new);
    }
}

#[cfg(test)]
pub fn switch_context(old: &mut CpuContext, new: &CpuContext) {
    *old = *new;
}
