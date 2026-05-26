//! Populate root filesystem at boot — /bin, /etc, /lib, embedded binaries.

use crate::console::Console;
use crate::fs::vfs::{mkdir_path, read_path, symlink_path, write_path};

pub fn populate() {
    let _ = mkdir_path(b"/bin");
    let _ = mkdir_path(b"/lib");
    let _ = mkdir_path(b"/etc");
    let _ = mkdir_path(b"/root");
    let _ = mkdir_path(b"/home");
    let _ = mkdir_path(b"/var");
    let _ = mkdir_path(b"/var/run");

    install_embedded();
    install_etc();

    Console::println("[rootfs] populated /bin /etc /lib /dev");
}

fn install_embedded() {
    if !crate::fs::embed::INIT.is_empty() {
        let _ = write_path(b"/bin/init", crate::fs::embed::INIT, true);
    }
    if !crate::fs::embed::SH.is_empty() {
        let _ = write_path(b"/bin/sh", crate::fs::embed::SH, true);
    }
    if !crate::fs::embed::UI.is_empty() {
        let _ = write_path(b"/bin/ui", crate::fs::embed::UI, true);
    }
    if !crate::fs::embed::BUSYBOX.is_empty() {
        let _ = write_path(b"/bin/busybox", crate::fs::embed::BUSYBOX, true);
        for applet in APPLETS {
            let link = alloc::format!("/bin/{applet}");
            let _ = symlink_path(link.as_bytes(), b"busybox");
        }
    }
    if !crate::fs::embed::LD_MUSL.is_empty() {
        let _ = write_path(b"/lib/ld-musl-x86_64.so.1", crate::fs::embed::LD_MUSL, true);
    }
    if read_path(b"/bin/sh").is_err() && read_path(b"/bin/init").is_ok() {
        let _ = symlink_path(b"/bin/sh", b"init");
    }
}

const APPLETS: &[&str] = &[
    "ls", "cat", "echo", "mkdir", "mount", "ps", "cp", "mv", "rm", "grep", "sh", "ash",
];

fn install_etc() {
    let passwd = b"root:x:0:0:root:/root:/bin/sh\n";
    let _ = write_path(b"/etc/passwd", passwd, true);
    let group = b"root:x:0:\n";
    let _ = write_path(b"/etc/group", group, true);
    let hosts = b"127.0.0.1 localhost\n";
    let _ = write_path(b"/etc/hosts", hosts, true);
    let init_tab = b"::sysinit:/bin/sh\n::respawn:/bin/sh\n";
    let _ = write_path(b"/etc/inittab", init_tab, true);
}
