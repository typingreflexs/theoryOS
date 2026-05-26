# Theory OS Userspace

Userspace binaries are embedded into the kernel at build time and installed on the root tmpfs at boot.

**Author:** typingreflexs — only error fixes assisted by Cursor.

## Quick build (init + sh)

```bash
make -C userspace          # needs gcc/binutils (Linux/WSL)
make kernel                # builds userspace then kernel
make iso && make run
```

## Components

| Path | Role |
|------|------|
| `init/init.S` | PID 1 — prints banner, execve `/bin/sh` |
| `sh/sh.S` | Minimal `$ ` prompt shell (read/write loop) |
| `musl/` | Cross-build scripts for musl libc |
| `busybox/` | Static busybox build when source cloned here |
| `build/` | Output: `init`, `sh`, optional `busybox` |

## musl port

1. Clone musl: `git clone https://github.com/bminor/musl userspace/musl/musl`
2. Install `x86_64-linux-musl-gcc` cross toolchain
3. `make -C userspace/musl`
4. Link programs with `-static` or dynamic + `/lib/ld-musl-x86_64.so.1`

The kernel ELF loader supports `PT_INTERP` for dynamic linking once the interpreter is installed at `/lib/ld-musl-x86_64.so.1`.

## busybox

1. Clone: `git clone https://git.busybox.net/busybox userspace/busybox/busybox`
2. `make -C userspace/busybox`
3. Rebuild kernel — busybox is embedded and symlinked as `ls`, `cat`, `echo`, etc.

## Boot flow

```
Limine → kernel_main → fs::init → rootfs::populate → sched → spawn_init_process()
  → PID 1 runs /bin/init → execve /bin/sh → interactive shell
```

## Syscalls used by musl/busybox (implemented)

`read`, `write`, `open`, `close`, `stat`, `fstat`, `lseek`, `mmap`, `munmap`, `mprotect`, `brk`, `execve`, `exit`, `exit_group`, `fork`, `wait4`, `getpid`, `getppid`, `getuid`, `uname`, `arch_prctl`, `set_tid_address`, `getcwd`, `chdir`, `access`, `readlink`, `getdents64`, `ioctl`, `fcntl`, `clock_gettime`, signals, IPC, sockets, and more.

Missing syscalls return `ENOSYS` — extend `kernel/src/syscall/table.rs` as needed.
