# Theory OS

A UNIX-like operating system written in Rust (`no_std`), booted via [Limine](https://github.com/Limine-Bootloader/Limine), targeting **x86-64** with an **ARM64 portability layer** for future ports.

**Author:** [typingreflexs](https://github.com/typingreflexs)  
**Assistance:** Only error fixes and debugging were assisted by Cursor. All design, architecture, and implementation are by typingreflexs.

---

## Features

| Area | What it does |
|------|----------------|
| **Boot** | Limine protocol, HHDM, memory map, framebuffer, SMP info |
| **CPU** | GDT/TSS, 256-vector IDT, LAPIC timer, IOAPIC, SMP bring-up |
| **Memory** | Buddy allocator, 4-level paging, slab heap, VMA/mmap, COW fork, ASLR, NUMA |
| **Processes** | PCB/TCB, fork/exec/exit/wait, ELF loader with relocations |
| **Scheduling** | CFS with red-black run queue, preemption, priority inheritance |
| **Syscalls** | SYSCALL/SYSRET fast path, Linux-compatible syscall numbers |
| **Filesystem** | VFS, tmpfs, procfs, devfs, sysfs, ext2, FAT32, embedded rootfs |
| **IPC** | Pipes, FIFOs, Unix sockets, message queues, shared memory, futex |
| **Network** | e1000/virtio-net, ARP, DHCP, DNS, TCP/UDP, HTTP client |
| **Security** | KPTI, stack canaries, capabilities, seccomp |
| **Desktop** | Framebuffer UI, taskbar, console shell, browser, settings |

---

## Project layout

```
THE/
├── Cargo.toml              # Workspace root
├── Makefile                # Build kernel, ISO, run in QEMU (Linux/WSL)
├── README.md               # This file
├── kernel/
│   ├── src/
│   │   ├── main.rs         # Entry point → arch entry
│   │   ├── lib.rs          # kernel_main boot sequence
│   │   ├── acpi/           # RSDP, MADT, FADT, DSDT, SRAT parsing
│   │   ├── arch/           # x86_64 + aarch64 portability
│   │   ├── boot/           # Limine requests → BootInfo
│   │   ├── console/        # Serial debug output
│   │   ├── fs/             # VFS and filesystem drivers
│   │   ├── input/          # PS/2 keyboard and mouse (polled)
│   │   ├── ipc/            # Inter-process communication
│   │   ├── mm/             # Physical memory, paging, heap, ELF
│   │   ├── net/            # Full network stack + drivers
│   │   ├── proc/           # Processes and threads
│   │   ├── sched/          # CFS scheduler and idle loop
│   │   ├── security/       # Hardening features
│   │   ├── signal/         # POSIX signal delivery
│   │   ├── sync/           # Spin locks and mutexes
│   │   ├── syscall/        # Syscall dispatch and handlers
│   │   └── video/          # Framebuffer desktop UI
│   ├── linker.ld           # Higher-half kernel @ 0xffffffff80000000
│   └── limine.conf         # Bootloader configuration
├── userspace/              # init, sh, musl/busybox build scripts
└── scripts/
    └── run.ps1             # Build + run in QEMU on Windows
```

---

## Requirements

### Windows (recommended script)

- [Rust nightly](https://rustup.rs/) with `rust-src` component
- [QEMU](https://www.qemu.org/) (`qemu-system-x86_64`)
- MSYS2 (for `xorriso` and ISO creation) — `C:\tools\msys64`

### Linux / WSL

- Rust nightly + `rust-src`
- QEMU, xorriso, git, make, gcc (for userspace asm)

---

## Build and run

### Windows

```powershell
powershell -ExecutionPolicy Bypass -File scripts\run.ps1
```

This builds the kernel, downloads Limine if needed, creates `build/theory.iso`, and launches QEMU with SDL display and user-mode networking (e1000).

Serial log: `build/qemu-serial.log`

### Linux / WSL

```bash
make kernel    # build userspace stubs + kernel
make iso       # create bootable ISO (needs Limine in build/limine/)
make run       # boot in QEMU
```

Manual kernel build:

```bash
cd kernel
cargo +nightly build --release \
  -Z build-std=core,compiler_builtins,alloc \
  --target x86_64-unknown-none
```

---

## Boot sequence

```
Limine bootloader
  → kernel entry (HHDM, serial)
  → arch::early_init()      GDT, TSS, legacy PIC off
  → mm::init()              phys mem, paging, heap
  → acpi::init()            MADT/FADT/DSDT
  → video::init()           framebuffer desktop
  → security::init()        KPTI, canaries, caps
  → fs::init()              mount VFS, populate /bin /etc
  → ipc::init()
  → net::init()             protocol stack (NIC probed on demand)
  → syscall::init()         SYSCALL MSRs
  → arch::apic_init()       LAPIC + IOAPIC
  → sched::start_cpu()      idle thread, UI loop, optional init process
```

---

## Desktop usage

After boot you get a graphical desktop with:

- **Start menu** — Console, Browser, Settings, Files, Network
- **Console** — built-in shell (`help`, `fetch`, `wifi`, `ping`, `open`)
- **Browser** — fetches `http://` pages (text-only, no TLS yet)
- **Network** — shows Ethernet/DHCP status (QEMU uses e1000, not real Wi-Fi)

### Browser

1. Start → **Browser**
2. Edit URL (default: `http://example.com`)
3. Press **Enter** — loads asynchronously without freezing the UI

### Console commands

| Command | Description |
|---------|-------------|
| `help` | List commands |
| `fetch http://host/path` | Download and print a web page |
| `wifi` / `net` | Show network status |
| `ping 10.0.2.2` | Send ICMP ping |
| `open browser` | Switch to an app |

---

## Kernel subsystems (quick reference)

### `arch/` — Hardware abstraction

- **x86_64:** GDT, TSS, IDT (256 vectors), LAPIC, IOAPIC, SMP trampoline, SYSCALL entry, context switch, XSAVE
- **aarch64:** Stub port layer behind `Arch` trait
- **memory.rs:** Physical ↔ virtual address helpers (HHDM)

### `mm/` — Memory management

- **phys/:** Frame bitmap + buddy allocator (orders 0–10)
- **paging/:** 4-level PML4, recursive self-map at index 510
- **heap/:** Slab allocator backing `GlobalAlloc`
- **vma/ + vm/:** Anonymous/file mappings, mmap/mprotect/munmap
- **cow/:** Copy-on-write for fork
- **elf/:** Parse, load, relocate user binaries

### `proc/` + `sched/` — Processes and scheduling

- **pcb.rs / thread.rs:** Process and thread control blocks
- **exec.rs:** ELF exec, spawn `/bin/init`
- **cfs.rs:** Completely Fair Scheduler weights and vruntime
- **idle.rs:** Per-CPU idle thread — polls input, network, redraws UI

### `fs/` — Filesystem

- **vfs/:** Inodes, dentries, mount table, path resolution
- **tmpfs/:** In-memory root filesystem
- **procfs/:** `/proc/cpuinfo`, `/proc/meminfo`, `/proc/fb`
- **embed.rs:** Userspace binaries baked into kernel at build time

### `net/` — Networking

- **drivers/:** Intel e1000, virtio-net PCI probe
- **dhcp.rs:** Non-blocking DHCP client
- **tcp/ + udp/:** Protocol state machines
- **http.rs:** Minimal HTTP/1.0 client for the browser

### `video/` — Graphics UI

- **framebuffer.rs:** Limine linear framebuffer
- **desktop.rs:** Wallpaper, taskbar, clock
- **apps.rs:** Window manager and launcher
- **browser.rs:** Async page loader
- **shell.rs:** In-kernel terminal emulator

### `syscall/` — System calls

- **table.rs:** Linux x86_64 syscall dispatch table
- **handlers/:** fs, proc, mm, ipc, socket, signal, security
- **uaccess.rs:** Safe copies to/from user memory

---

## Userspace

See [userspace/README.md](userspace/README.md) for building `init`, `sh`, musl, and busybox.

Boot flow after scheduler starts:

```
PID 1 (/bin/init) → execve /bin/sh → interactive shell
```

---

## Configuration

| File | Purpose |
|------|---------|
| `kernel/limine.conf` | Limine: kernel path, framebuffer, SMP |
| `kernel/rust-toolchain.toml` | Nightly toolchain pin |
| `kernel/.cargo/config.toml` | Build target defaults |
| `scripts/run.ps1` | QEMU args: 512M RAM, e1000 + user netdev |

---

## Known limits

- **HTTPS** not implemented (browser is HTTP-only)
- **Wi-Fi** — no 802.11 driver; QEMU uses Ethernet (e1000)
- **HTML rendering** — text extraction only, tags stripped
- **SMP** — APIC timer IRQs deferred; UI runs on BSP idle thread
- **ARM64** — trait layer exists; boot not complete

---

## License

Add your license here (e.g. MIT, GPL-3.0).

---

## Contributing

This is a personal OS project by **typingreflexs**. Pull requests welcome on [github.com/typingreflexs/theoryOS](https://github.com/typingreflexs/theoryOS).
