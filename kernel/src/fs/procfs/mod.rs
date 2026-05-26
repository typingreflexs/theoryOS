//! procfs — virtual `/proc` hierarchy with process and system info.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use crate::fs::vfs::superblock::{DirEntry, FileSystem, FsError};
use crate::fs::vfs::inode::{FileType, InodeAttr, InodeId, InodeMode};
use crate::proc::{self, table, Pid};

const ROOT_INO: u64 = 1;
const SELF_INO: u64 = 2;
const CPUINFO_INO: u64 = 3;
const MEMINFO_INO: u64 = 4;
const FB_INO: u64 = 5;
const PID_BASE: u64 = 1000;

struct ProcFsInner {
    mount_id: u32,
}

pub struct ProcFs {
    inner: Mutex<ProcFsInner>,
}

impl ProcFs {
    pub fn new(mount_id: u32) -> Self {
        Self {
            inner: Mutex::new(ProcFsInner { mount_id }),
        }
    }

    fn mount_id(&self) -> u32 {
        self.inner.lock().mount_id
    }

    fn static_attr(ino: u64, file_type: FileType, size: u64) -> InodeAttr {
        InodeAttr {
            ino,
            mode: InodeMode::default_file(),
            file_type,
            size,
            nlink: 1,
            generation: 1,
            rdev: 0,
        }
    }

    fn pid_from_ino(ino: u64) -> Option<Pid> {
        if ino >= PID_BASE {
            Some(Pid::new((ino - PID_BASE) as u32))
        } else {
            None
        }
    }

    fn ino_for_pid(pid: Pid) -> u64 {
        PID_BASE + pid.as_u32() as u64
    }

    fn generate_file(ino: u64) -> Vec<u8> {
        match ino {
            SELF_INO => {
                let pid = proc::current_thread(|t| t.pid)
                    .unwrap_or(Pid::KERNEL)
                    .as_u32();
                format!("{pid}\n").into_bytes()
            }
            CPUINFO_INO => b"processor\t: 0\nvendor_id\t: TheoryOS\nmodel name\t: Theory CPU\n".to_vec(),
            MEMINFO_INO => {
                let stats = crate::mm::phys::stats();
                format!(
                    "MemTotal:       {} kB\nMemFree:        {} kB\n",
                    stats.total_frames * 4,
                    stats.free_frames * 4
                )
                .into_bytes()
            }
            FB_INO => crate::video::proc_fb_line().into_bytes(),
            _ => {
                if let Some(pid) = Self::pid_from_ino(ino) {
                    let base = Self::ino_for_pid(pid);
                    if ino == base + 1 {
                        table::with_process(pid, |p| {
                            format!(
                                "Name:\tprocess-{}\nPid:\t{}\nPPid:\t{}\nState:\t{:?}\n",
                                pid.as_u32(),
                                p.pid.as_u32(),
                                p.parent.as_u32(),
                                p.state
                            )
                            .into_bytes()
                        })
                        .unwrap_or_else(|| b"Name:\t<unknown>\n".to_vec())
                    } else if ino == base + 2 {
                        Self::format_maps(pid)
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn format_maps(pid: Pid) -> Vec<u8> {
        table::with_process(pid, |p| {
            let Some(ref space) = p.address_space else {
                return Vec::new();
            };
            let mut out = String::new();
            for vma in space.vma.iter() {
                let prot = vma_prot_str(vma.prot);
                let kind = match vma.kind {
                    crate::mm::vma::VmaKind::Heap => "[heap]",
                    crate::mm::vma::VmaKind::Stack => "[stack]",
                    crate::mm::vma::VmaKind::Framebuffer => "[fb]",
                    crate::mm::vma::VmaKind::File => "",
                    _ => "",
                };
                out.push_str(&format!(
                    "{:016x}-{:016x} {} 00000000 00:00 0 {}\n",
                    vma.start, vma.end, prot, kind
                ));
            }
            out.into_bytes()
        })
        .unwrap_or_default()
    }

    fn list_pids() -> Vec<Pid> {
        let mut pids = Vec::new();
        for i in 0..table::MAX_PROCESSES {
            let pid = Pid::new(i as u32);
            if table::with_process(pid, |_| ()).is_some() {
                pids.push(pid);
            }
        }
        pids
    }
}

fn vma_prot_str(prot: crate::mm::permissions::ProtFlags) -> &'static str {
    let r = if prot.contains(crate::mm::permissions::ProtFlags::READ) {
        'r'
    } else {
        '-'
    };
    let w = if prot.contains(crate::mm::permissions::ProtFlags::WRITE) {
        'w'
    } else {
        '-'
    };
    let x = if prot.contains(crate::mm::permissions::ProtFlags::EXEC) {
        'x'
    } else {
        '-'
    };
    match (r, w, x) {
        ('r', 'w', 'x') => "rwxp",
        ('r', 'w', '-') => "rw-p",
        ('r', '-', 'x') => "r-xp",
        ('r', '-', '-') => "r--p",
        ('-', '-', 'x') => "--xp",
        ('-', 'w', 'x') => "-wxp",
        ('-', 'w', '-') => "-w-p",
        _ => "----",
    }
}

impl FileSystem for ProcFs {
    fn fs_type(&self) -> &'static str {
        "procfs"
    }

    fn root(&self) -> InodeId {
        InodeId::new(self.mount_id(), ROOT_INO)
    }

    fn getattr(&self, inode: InodeId) -> Result<InodeAttr, FsError> {
        if inode.mount != self.mount_id() {
            return Err(FsError::NotFound);
        }
        let attr = match inode.ino {
            ROOT_INO => InodeAttr {
                ino: ROOT_INO,
                mode: InodeMode::default_dir(),
                file_type: FileType::Directory,
                size: 0,
                nlink: 2,
                generation: 1,
                rdev: 0,
            },
            SELF_INO => Self::static_attr(SELF_INO, FileType::Regular, 8),
            CPUINFO_INO => Self::static_attr(CPUINFO_INO, FileType::Regular, 64),
            MEMINFO_INO => Self::static_attr(MEMINFO_INO, FileType::Regular, 64),
            FB_INO => Self::static_attr(FB_INO, FileType::Regular, 32),
            ino if Self::pid_from_ino(ino).is_some() => {
                let pid = Self::pid_from_ino(ino).unwrap();
                let base = Self::ino_for_pid(pid);
                if ino == base {
                    InodeAttr {
                        ino: base,
                        mode: InodeMode::default_dir(),
                        file_type: FileType::Directory,
                        size: 0,
                        nlink: 2,
                        generation: 1,
                        rdev: 0,
                    }
                } else {
                    Self::static_attr(ino, FileType::Regular, 256)
                }
            }
            _ => return Err(FsError::NotFound),
        };
        Ok(attr)
    }

    fn lookup(&self, dir: InodeId, name: &[u8]) -> Result<InodeId, FsError> {
        let name = core::str::from_utf8(name).map_err(|_| FsError::Invalid)?;
        let mid = self.mount_id();
        if dir.ino == ROOT_INO {
            match name {
                "self" => return Ok(InodeId::new(mid, SELF_INO)),
                "cpuinfo" => return Ok(InodeId::new(mid, CPUINFO_INO)),
                "meminfo" => return Ok(InodeId::new(mid, MEMINFO_INO)),
                "fb" => return Ok(InodeId::new(mid, FB_INO)),
                s => {
                    if let Ok(pid) = s.parse::<u32>() {
                        let pid = Pid::new(pid);
                        if table::with_process(pid, |_| ()).is_some() {
                            return Ok(InodeId::new(mid, Self::ino_for_pid(pid)));
                        }
                    }
                }
            }
            return Err(FsError::NotFound);
        }
        if let Some(pid) = Self::pid_from_ino(dir.ino) {
            if table::with_process(pid, |_| ()).is_none() {
                return Err(FsError::NotFound);
            }
            match name {
                "status" => return Ok(InodeId::new(mid, dir.ino + 1)),
                "maps" => return Ok(InodeId::new(mid, dir.ino + 2)),
                _ => return Err(FsError::NotFound),
            }
        }
        Err(FsError::NotDir)
    }

    fn create(
        &self,
        _: InodeId,
        _: &[u8],
        _: FileType,
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn mkdir(&self, _: InodeId, _: &[u8], _: InodeMode) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn unlink(&self, _: InodeId, _: &[u8]) -> Result<(), FsError> {
        Err(FsError::AccessDenied)
    }

    fn read(&self, inode: InodeId, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let data = Self::generate_file(inode.ino);
        if offset as usize >= data.len() {
            return Ok(0);
        }
        let start = offset as usize;
        let end = (start + buf.len()).min(data.len());
        buf[..end - start].copy_from_slice(&data[start..end]);
        Ok(end - start)
    }

    fn write(&self, _: InodeId, _: u64, _: &[u8]) -> Result<usize, FsError> {
        Err(FsError::AccessDenied)
    }

    fn readdir(&self, dir: InodeId, index: u64, entry: &mut DirEntry) -> Result<bool, FsError> {
        let mid = self.mount_id();
        if dir.ino == ROOT_INO {
            static ROOT_NAMES: [&str; 4] = ["self", "cpuinfo", "meminfo", "fb"];
            if index < ROOT_NAMES.len() as u64 {
                let name = ROOT_NAMES[index as usize];
                entry.name_len = name.len();
                entry.name[..name.len()].copy_from_slice(name.as_bytes());
                entry.inode = match name {
                    "self" => InodeId::new(mid, SELF_INO),
                    "cpuinfo" => InodeId::new(mid, CPUINFO_INO),
                    "meminfo" => InodeId::new(mid, MEMINFO_INO),
                    _ => InodeId::new(mid, FB_INO),
                };
                entry.file_type = FileType::Regular;
                return Ok(true);
            }
            let idx = index - ROOT_NAMES.len() as u64;
            let pids = Self::list_pids();
            if (idx as usize) < pids.len() {
                let pid = pids[idx as usize];
                let name = format!("{}", pid.as_u32());
                entry.name_len = name.len().min(256);
                entry.name[..entry.name_len].copy_from_slice(&name.as_bytes()[..entry.name_len]);
                entry.inode = InodeId::new(mid, Self::ino_for_pid(pid));
                entry.file_type = FileType::Directory;
                return Ok(true);
            }
            return Ok(false);
        }
        if let Some(_pid) = Self::pid_from_ino(dir.ino) {
            static PID_NAMES: [&str; 2] = ["status", "maps"];
            if index >= PID_NAMES.len() as u64 {
                return Ok(false);
            }
            let name = PID_NAMES[index as usize];
            entry.name_len = name.len();
            entry.name[..name.len()].copy_from_slice(name.as_bytes());
            entry.inode = InodeId::new(mid, dir.ino + index + 1);
            entry.file_type = FileType::Regular;
            return Ok(true);
        }
        Err(FsError::NotDir)
    }

    fn readlink(&self, _: InodeId, _: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSymlink)
    }

    fn symlink(
        &self,
        _: InodeId,
        _: &[u8],
        _: &[u8],
        _: InodeMode,
    ) -> Result<InodeId, FsError> {
        Err(FsError::AccessDenied)
    }

    fn truncate(&self, _: InodeId, _: u64) -> Result<(), FsError> {
        Err(FsError::AccessDenied)
    }
}
