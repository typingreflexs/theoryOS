//! Named FIFOs — path-keyed pipes backed by tmpfs rdev encoding.

use spin::Mutex;

use super::pipe::{Pipe, PipeId};

const MAX_FIFOS: usize = 32;

struct FifoEntry {
    path: [u8; 256],
    path_len: usize,
    pipe: PipeId,
}

static FIFOS: Mutex<[Option<FifoEntry>; MAX_FIFOS]> = Mutex::new([const { None }; MAX_FIFOS]);

pub fn mkfifo(path: &[u8]) -> Result<PipeId, ()> {
    let (read_id, _write_id) = Pipe::create()?;
    let mut fifos = FIFOS.lock();
    for slot in fifos.iter_mut() {
        if slot.is_none() {
            let mut entry = FifoEntry {
                path: [0; 256],
                path_len: path.len().min(256),
                pipe: read_id,
            };
            entry.path[..entry.path_len].copy_from_slice(&path[..entry.path_len]);
            *slot = Some(entry);
            return Ok(read_id);
        }
    }
    Err(())
}

pub fn lookup(path: &[u8]) -> Option<PipeId> {
    let fifos = FIFOS.lock();
    for entry in fifos.iter().flatten() {
        if entry.path_len == path.len() && &entry.path[..entry.path_len] == path {
            return Some(entry.pipe);
        }
    }
    None
}

pub fn register(path: &[u8], pipe: PipeId) {
    if lookup(path).is_some() {
        return;
    }
    let mut fifos = FIFOS.lock();
    for slot in fifos.iter_mut() {
        if slot.is_none() {
            let mut entry = FifoEntry {
                path: [0; 256],
                path_len: path.len().min(256),
                pipe,
            };
            entry.path[..entry.path_len].copy_from_slice(&path[..entry.path_len]);
            *slot = Some(entry);
            return;
        }
    }
}

pub fn encode_rdev(pipe: PipeId) -> u32 {
    0x1000 | pipe.as_u32()
}

pub fn decode_rdev(rdev: u32) -> Option<PipeId> {
    if rdev & 0x1000 != 0 {
        Some(PipeId::new(rdev & 0x0FFF))
    } else {
        None
    }
}
