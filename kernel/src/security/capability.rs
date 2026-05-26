//! Linux-compatible capability bits for privilege checks.

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct CapSet: u64 {
        const CHOWN = 1 << 0;
        const DAC_OVERRIDE = 1 << 1;
        const DAC_READ_SEARCH = 1 << 2;
        const FOWNER = 1 << 3;
        const KILL = 1 << 5;
        const SETGID = 1 << 6;
        const SETUID = 1 << 7;
        const NET_BIND_SERVICE = 1 << 10;
        const NET_RAW = 1 << 13;
        const SYS_CHROOT = 1 << 18;
        const SYS_PTRACE = 1 << 19;
        const SYS_ADMIN = 1 << 21;
        const SYS_BOOT = 1 << 22;
        const SYS_NICE = 1 << 23;
        const SYS_RESOURCE = 1 << 24;
        const SYS_TIME = 1 << 25;
        const SYS_TTY_CONFIG = 1 << 26;
        const MKNOD = 1 << 27;
        const LEASE = 1 << 28;
        const AUDIT_WRITE = 1 << 29;
        const AUDIT_CONTROL = 1 << 30;
        const SETPCAP = 1 << 31;
        const MAC_OVERRIDE = 1 << 32;
        const MAC_ADMIN = 1 << 33;
        const SYSLOG = 1 << 34;
        const WAKE_ALARM = 1 << 35;
        const BLOCK_SUSPEND = 1 << 36;
        const AUDIT_READ = 1 << 37;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub permitted: CapSet,
    pub effective: CapSet,
    pub inheritable: CapSet,
}

impl Credentials {
    pub fn root() -> Self {
        let all = CapSet::CHOWN
            | CapSet::DAC_OVERRIDE
            | CapSet::KILL
            | CapSet::NET_BIND_SERVICE
            | CapSet::SYS_ADMIN
            | CapSet::SYS_PTRACE
            | CapSet::SYS_BOOT;
        Self {
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            permitted: all,
            effective: all,
            inheritable: all,
        }
    }

    pub fn user() -> Self {
        Self {
            uid: 1000,
            gid: 1000,
            euid: 1000,
            egid: 1000,
            permitted: CapSet::empty(),
            effective: CapSet::empty(),
            inheritable: CapSet::empty(),
        }
    }

    pub fn is_privileged(&self) -> bool {
        self.euid == 0
    }

    pub fn capable(&self, cap: CapSet) -> bool {
        if self.euid == 0 {
            return true;
        }
        self.effective.contains(cap)
    }

    /// Effective caps after fork: inheritable & permitted become effective.
    pub fn fork(&self) -> Self {
        let effective = self.inheritable & self.permitted;
        Self {
            uid: self.uid,
            gid: self.gid,
            euid: self.euid,
            egid: self.egid,
            permitted: self.permitted,
            effective,
            inheritable: self.inheritable,
        }
    }

    pub fn can_signal(&self, target: &Self) -> bool {
        if self.capable(CapSet::KILL) {
            return true;
        }
        self.euid == target.euid
            || self.euid == target.uid
            || self.uid == target.euid
            || self.uid == target.uid
    }
}
