use std::env;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let target = env::var("TARGET").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if target.starts_with("x86_64") && target.contains("none") {
        let inc = out_dir.join("interrupt_stubs.inc");
        generate_interrupt_stubs(&inc);
        println!("cargo:rustc-env=THEORY_INTERRUPT_INC={}", inc.display());

        let ctx = out_dir.join("context_switch.inc");
        generate_context_switch(&ctx);
        println!("cargo:rustc-env=THEORY_CONTEXT_INC={}", ctx.display());

        let sc = out_dir.join("syscall_entry.inc");
        generate_syscall_entry(&sc);
        println!("cargo:rustc-env=THEORY_SYSCALL_INC={}", sc.display());

        println!(
            "cargo:rustc-link-arg=-T{}",
            manifest_dir.join("linker.ld").display()
        );
        println!("cargo:rustc-link-arg=--entry=_start");
        println!("cargo:rerun-if-changed=linker.ld");
    } else if target.starts_with("aarch64") {
        println!(
            "cargo:rustc-link-arg=-T{}",
            manifest_dir.join("linker-aarch64.ld").display()
        );
        println!("cargo:rustc-link-arg=--entry=_start");
        println!("cargo:rerun-if-changed=linker-aarch64.ld");
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../userspace");

    generate_rootfs_embed(&out_dir, &manifest_dir);
}

fn generate_rootfs_embed(out_dir: &Path, manifest_dir: &Path) {
    let workspace = manifest_dir.parent().expect("workspace root");
    let userspace_dir = workspace.join("userspace");
    let build_dir = userspace_dir.join("build");

    let _ = std::process::Command::new("make")
        .current_dir(&userspace_dir)
        .status();

    let embed_path = out_dir.join("rootfs_embed.rs");
    let mut file = File::create(&embed_path).expect("rootfs_embed.rs");

    write_embed_bytes(&out_dir, &mut file, "INIT", &build_dir.join("init"), fallback_init());
    write_embed_bytes(&out_dir, &mut file, "SH", &build_dir.join("sh"), fallback_sh());
    write_embed_bytes(&out_dir, &mut file, "UI", &build_dir.join("ui"), fallback_ui());
    write_embed_bytes(&out_dir, &mut file, "BUSYBOX", &build_dir.join("busybox"), &[]);
    write_embed_bytes(
        &out_dir,
        &mut file,
        "LD_MUSL",
        &build_dir.join("lib/ld-musl-x86_64.so.1"),
        &[],
    );
}

fn write_embed_bytes(out_dir: &Path, file: &mut File, name: &str, path: &Path, fallback: &[u8]) {
    let data = if path.exists() {
        std::fs::read(path).unwrap_or_else(|_| fallback.to_vec())
    } else {
        fallback.to_vec()
    };
    let bin_path = out_dir.join(format!("{}.bin", name.to_lowercase()));
    std::fs::write(&bin_path, &data).expect("write embed bin");
    writeln!(
        file,
        "pub static {}: &[u8] = include_bytes!(r\"{}\");",
        name,
        bin_path.display()
    )
    .unwrap();
}

/// Minimal static ET_EXEC ELF at 0x400000 — write + execve fallback.
fn fallback_init() -> &'static [u8] {
    &[
        0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xc7,
        0xc0, 0x01, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00, 0x48, 0x8d,
        0x35, 0x0a, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc2, 0x0e, 0x00, 0x00, 0x00, 0x0f, 0x05,
        0x48, 0xc7, 0xc0, 0x3b, 0x00, 0x00, 0x00, 0x48, 0x8d, 0x3d, 0x0a, 0x00, 0x00, 0x00,
        0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x0f, 0x05, 0x48, 0xc7, 0xc0, 0x3c, 0x00, 0x00,
        0x00, 0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x54, 0x68, 0x65, 0x6f,
        0x72, 0x79, 0x20, 0x4f, 0x53, 0x0a, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x75, 0x69, 0x00,
    ]
}

/// Minimal UI — nanosleep loop; kernel timer owns clock redraw.
fn fallback_ui() -> &'static [u8] {
    &[
        0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x8d,
        0x3d, 0x0a, 0x00, 0x00, 0x00, 0x48, 0x31, 0xf6, 0x48, 0xc7, 0xc0, 0x23, 0x00, 0x00,
        0x00, 0x0f, 0x05, 0xeb, 0xef, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x1d, 0xcd, 0x65, 0x00,
    ]
}

fn fallback_sh() -> &'static [u8] {
    &[
        0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xc7,
        0xc0, 0x01, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00, 0x48, 0x8d,
        0x35, 0x2a, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc2, 0x02, 0x00, 0x00, 0x00, 0x0f, 0x05,
        0x48, 0x31, 0xc0, 0x48, 0x31, 0xff, 0x48, 0x8d, 0x35, 0x2a, 0x00, 0x00, 0x00, 0x48,
        0xc7, 0xc2, 0x7f, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x85, 0xc0, 0x7e, 0xe0, 0x48,
        0x89, 0xc2, 0x48, 0xc7, 0xc0, 0x01, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc7, 0x01, 0x00,
        0x00, 0x00, 0x48, 0x8d, 0x35, 0x10, 0x00, 0x00, 0x00, 0x0f, 0x05, 0xeb, 0xc4, 0x24,
        0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]
}

fn generate_interrupt_stubs(path: &Path) {
    let mut file = File::create(path).expect("failed to create interrupt stub assembly");

    const HAS_ERROR_CODE: [bool; 32] = [
        false, false, true, false, true, true, true, false, true, true, true, true, true, true,
        true, false, true, true, true, true, true, true, true, true, true, true, true, true,
        true, true, true, true,
    ];

    writeln!(file, ".intel_syntax noprefix").unwrap();
    writeln!(file, ".section .text.interrupt_stubs, \"ax\", @progbits").unwrap();
    writeln!(file, ".global theory_interrupt_common").unwrap();
    writeln!(file, ".extern theory_interrupt_dispatch").unwrap();

    for vector in 0..=255u8 {
        let name = format!("theory_vector_{vector}");
        writeln!(file).unwrap();
        writeln!(file, ".global {name}").unwrap();
        writeln!(file, ".type {name}, @function").unwrap();
        writeln!(file, "{name}:").unwrap();
        if vector < 32 && HAS_ERROR_CODE[vector as usize] {
            writeln!(file, "    push {vector}").unwrap();
        } else {
            writeln!(file, "    push 0").unwrap();
            writeln!(file, "    push {vector}").unwrap();
        }
        writeln!(file, "    jmp theory_interrupt_common").unwrap();
    }

    writeln!(file).unwrap();
    writeln!(file, "theory_interrupt_common:").unwrap();
    writeln!(file, "    push rax").unwrap();
    writeln!(file, "    push rcx").unwrap();
    writeln!(file, "    push rdx").unwrap();
    writeln!(file, "    push rbx").unwrap();
    writeln!(file, "    push rbp").unwrap();
    writeln!(file, "    push rsi").unwrap();
    writeln!(file, "    push rdi").unwrap();
    writeln!(file, "    push r8").unwrap();
    writeln!(file, "    push r9").unwrap();
    writeln!(file, "    push r10").unwrap();
    writeln!(file, "    push r11").unwrap();
    writeln!(file, "    push r12").unwrap();
    writeln!(file, "    push r13").unwrap();
    writeln!(file, "    push r14").unwrap();
    writeln!(file, "    push r15").unwrap();
    writeln!(file, "    mov rdi, rsp").unwrap();
    writeln!(file, "    sub rsp, 8").unwrap();
    writeln!(file, "    call theory_interrupt_dispatch").unwrap();
    writeln!(file, "    add rsp, 8").unwrap();
    writeln!(file, "    pop r15").unwrap();
    writeln!(file, "    pop r14").unwrap();
    writeln!(file, "    pop r13").unwrap();
    writeln!(file, "    pop r12").unwrap();
    writeln!(file, "    pop r11").unwrap();
    writeln!(file, "    pop r10").unwrap();
    writeln!(file, "    pop r9").unwrap();
    writeln!(file, "    pop r8").unwrap();
    writeln!(file, "    pop rdi").unwrap();
    writeln!(file, "    pop rsi").unwrap();
    writeln!(file, "    pop rbp").unwrap();
    writeln!(file, "    pop rbx").unwrap();
    writeln!(file, "    pop rdx").unwrap();
    writeln!(file, "    pop rcx").unwrap();
    writeln!(file, "    pop rax").unwrap();
    writeln!(file, "    add rsp, 16").unwrap();
    writeln!(file, "    iretq").unwrap();

    writeln!(file).unwrap();
    writeln!(file, ".section .rodata.interrupt_table, \"a\", @progbits").unwrap();
    writeln!(file, ".global theory_vector_table").unwrap();
    writeln!(file, "theory_vector_table:").unwrap();
    for vector in 0..=255u8 {
        writeln!(file, "    .quad theory_vector_{vector}").unwrap();
    }
}

fn generate_context_switch(path: &Path) {
    let mut file = File::create(path).expect("failed to create context switch assembly");

    writeln!(file, ".intel_syntax noprefix").unwrap();
    writeln!(file, ".section .text.context_switch, \"ax\", @progbits").unwrap();
    writeln!(file, ".global theory_context_switch").unwrap();
    writeln!(file, ".type theory_context_switch, @function").unwrap();
    writeln!(file, "theory_context_switch:").unwrap();
    writeln!(file, "    // rdi = old *mut CpuContext, rsi = new *const CpuContext").unwrap();
    writeln!(file, "    push rbp").unwrap();
    writeln!(file, "    push rbx").unwrap();
    writeln!(file, "    push r12").unwrap();
    writeln!(file, "    push r13").unwrap();
    writeln!(file, "    push r14").unwrap();
    writeln!(file, "    push r15").unwrap();
    writeln!(file, "    mov r8, rdi").unwrap();
    writeln!(file, "    mov r9, rsi").unwrap();
    // Pop saved registers into old context
    for (reg, off) in [
        ("r15", 0),
        ("r14", 8),
        ("r13", 16),
        ("r12", 24),
        ("rbx", 88),
    ] {
        let _ = reg;
        writeln!(file, "    pop rax").unwrap();
        writeln!(file, "    mov [r8 + {off}], rax").unwrap();
    }
    writeln!(file, "    mov [r8 + 80], rbp").unwrap();
    writeln!(file, "    lea rax, [rip + 1f]").unwrap();
    writeln!(file, "    mov [r8 + 120], rax").unwrap();
    writeln!(file, "    mov [r8 + 144], rsp").unwrap();
    writeln!(file, "    pushfq").unwrap();
    writeln!(file, "    pop rax").unwrap();
    writeln!(file, "    mov [r8 + 136], rax").unwrap();
    // Restore from new context via iretq frame
    writeln!(file, "    mov rsp, [r9 + 144]").unwrap();
    writeln!(file, "    push qword ptr [r9 + 152]").unwrap();
    writeln!(file, "    push qword ptr [r9 + 144]").unwrap();
    writeln!(file, "    push qword ptr [r9 + 136]").unwrap();
    writeln!(file, "    push qword ptr [r9 + 128]").unwrap();
    writeln!(file, "    push qword ptr [r9 + 120]").unwrap();
    writeln!(file, "    mov r15, [r9 + 0]").unwrap();
    writeln!(file, "    mov r14, [r9 + 8]").unwrap();
    writeln!(file, "    mov r13, [r9 + 16]").unwrap();
    writeln!(file, "    mov r12, [r9 + 24]").unwrap();
    writeln!(file, "    mov r11, [r9 + 32]").unwrap();
    writeln!(file, "    mov r10, [r9 + 40]").unwrap();
    writeln!(file, "    mov r8, [r9 + 56]").unwrap();
    writeln!(file, "    mov r9, [r9 + 48]").unwrap();
    writeln!(file, "    mov rdi, [r9 + 64]").unwrap();
    writeln!(file, "    mov rsi, [r9 + 72]").unwrap();
    writeln!(file, "    mov rbp, [r9 + 80]").unwrap();
    writeln!(file, "    mov rbx, [r9 + 88]").unwrap();
    writeln!(file, "    mov rdx, [r9 + 96]").unwrap();
    writeln!(file, "    mov rcx, [r9 + 104]").unwrap();
    writeln!(file, "    mov rax, [r9 + 112]").unwrap();
    writeln!(file, "    iretq").unwrap();
    writeln!(file, "1:").unwrap();
    writeln!(file, "    pop r15").unwrap();
    writeln!(file, "    pop r14").unwrap();
    writeln!(file, "    pop r13").unwrap();
    writeln!(file, "    pop r12").unwrap();
    writeln!(file, "    pop rbx").unwrap();
    writeln!(file, "    pop rbp").unwrap();
    writeln!(file, "    ret").unwrap();
}

fn generate_syscall_entry(path: &Path) {
    let mut file = File::create(path).expect("failed to create syscall entry assembly");

    writeln!(file, ".intel_syntax noprefix").unwrap();
    writeln!(file, ".section .text.syscall_entry, \"ax\", @progbits").unwrap();
    writeln!(file, ".global theory_syscall_entry").unwrap();
    writeln!(file, ".extern theory_syscall_dispatch").unwrap();
    writeln!(file, ".extern theory_syscall_kernel_stack").unwrap();
    writeln!(file, ".extern theory_kpti_enter").unwrap();
    writeln!(file, ".extern theory_kpti_exit").unwrap();
    writeln!(file, ".type theory_syscall_entry, @function").unwrap();
    writeln!(file, "theory_syscall_entry:").unwrap();
    writeln!(file, "    call theory_kpti_enter").unwrap();
    // RCX=user RIP, R11=user RFLAGS, RSP=user stack (SYSCALL clobbers RCX/R11)
    writeln!(file, "    mov r10, rsp").unwrap(); // save user rsp in r10 (arg4 reg)
    writeln!(file, "    call theory_syscall_kernel_stack").unwrap();
    writeln!(file, "    mov rsp, rax").unwrap();
    writeln!(file, "    push r10").unwrap(); // user_rsp
    writeln!(file, "    push r11").unwrap(); // user_rflags
    writeln!(file, "    push rcx").unwrap(); // user_rip
    writeln!(file, "    push r15").unwrap();
    writeln!(file, "    push r14").unwrap();
    writeln!(file, "    push r13").unwrap();
    writeln!(file, "    push r12").unwrap();
    writeln!(file, "    push r11").unwrap();
    writeln!(file, "    push r10").unwrap();
    writeln!(file, "    push r9").unwrap();
    writeln!(file, "    push r8").unwrap();
    writeln!(file, "    push rbp").unwrap();
    writeln!(file, "    push rbx").unwrap();
    writeln!(file, "    push rdx").unwrap();
    writeln!(file, "    push rsi").unwrap();
    writeln!(file, "    push rdi").unwrap();
    writeln!(file, "    push rax").unwrap();
    writeln!(file, "    mov rdi, rsp").unwrap();
    writeln!(file, "    call theory_syscall_dispatch").unwrap();
    writeln!(file, "    pop rax").unwrap();
    writeln!(file, "    pop rdi").unwrap();
    writeln!(file, "    pop rsi").unwrap();
    writeln!(file, "    pop rdx").unwrap();
    writeln!(file, "    pop rbx").unwrap();
    writeln!(file, "    pop rbp").unwrap();
    writeln!(file, "    pop r8").unwrap();
    writeln!(file, "    pop r9").unwrap();
    writeln!(file, "    pop r10").unwrap();
    writeln!(file, "    pop r11").unwrap();
    writeln!(file, "    pop r12").unwrap();
    writeln!(file, "    pop r13").unwrap();
    writeln!(file, "    pop r14").unwrap();
    writeln!(file, "    pop r15").unwrap();
    writeln!(file, "    pop rcx").unwrap(); // user_rip -> RCX for sysret
    writeln!(file, "    pop r11").unwrap(); // user_rflags -> R11 for sysret
    writeln!(file, "    pop rsp").unwrap(); // user_rsp
    writeln!(file, "    call theory_kpti_exit").unwrap();
    writeln!(file, "    sysretq").unwrap();
}
