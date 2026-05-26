//! Network address types.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub const BROADCAST: Self = Self([0xFF; 6]);
    pub const ZERO: Self = Self([0; 6]);

    pub fn from_bytes(b: [u8; 6]) -> Self {
        Self(b)
    }

    pub fn is_broadcast(self) -> bool {
        self.0.iter().all(|&b| b == 0xFF)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ANY: Self = Self([0, 0, 0, 0]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);

    pub fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    pub fn from_u32(v: u32) -> Self {
        Self([(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8])
    }

    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn is_unspecified(self) -> bool {
        self.0 == [0; 4]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Ipv6Addr(pub [u8; 16]);

impl Ipv6Addr {
    pub const UNSPECIFIED: Self = Self([0; 16]);
    pub const LOOPBACK: Self = Self([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IpAddr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl Default for IpAddr {
    fn default() -> Self {
        Self::V4(Ipv4Addr::ANY)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockAddrIn6 {
    pub sin6_family: u16,
    pub sin6_port: u16,
    pub sin6_flowinfo: u32,
    pub sin6_addr: [u8; 16],
    pub sin6_scope_id: u32,
}

pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;
pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;

pub fn parse_sockaddr_in(buf: &[u8]) -> Option<(IpAddr, u16)> {
    if buf.len() < core::mem::size_of::<SockAddrIn>() {
        return None;
    }
    let sa = unsafe { &*(buf.as_ptr() as *const SockAddrIn) };
    if sa.sin_family != AF_INET {
        return None;
    }
    Some((IpAddr::V4(Ipv4Addr::from_u32(sa.sin_addr)), sa.sin_port))
}

pub fn write_sockaddr_in(addr: Ipv4Addr, port: u16, buf: &mut [u8]) -> usize {
    if buf.len() < core::mem::size_of::<SockAddrIn>() {
        return 0;
    }
    let sa = SockAddrIn {
        sin_family: AF_INET,
        sin_port: port,
        sin_addr: addr.to_u32(),
        sin_zero: [0; 8],
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            &sa as *const SockAddrIn as *const u8,
            buf.as_mut_ptr(),
            core::mem::size_of::<SockAddrIn>(),
        );
    }
    core::mem::size_of::<SockAddrIn>()
}
