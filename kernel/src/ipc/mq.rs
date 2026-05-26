//! SysV-style message queues (msgget, msgsnd, msgrcv, msgctl).

use spin::Mutex;

const MAX_QUEUES: usize = 32;
const MAX_MSG_SIZE: usize = 4096;
const MAX_MSGS: usize = 64;

#[derive(Clone, Copy)]
struct Message {
    mtype: i64,
    len: usize,
    data: [u8; MAX_MSG_SIZE],
}

struct MsgQueue {
    id: i32,
    key: i32,
    owner: u32,
    msgs: [Option<Message>; MAX_MSGS],
    msg_count: usize,
    max_bytes: usize,
    current_bytes: usize,
}

impl MsgQueue {
    const fn new(id: i32, key: i32, owner: u32) -> Self {
        Self {
            id,
            key,
            owner,
            msgs: [None; MAX_MSGS],
            msg_count: 0,
            max_bytes: MAX_MSG_SIZE * MAX_MSGS,
            current_bytes: 0,
        }
    }
}

static QUEUES: Mutex<[Option<MsgQueue>; MAX_QUEUES]> = Mutex::new([const { None }; MAX_QUEUES]);
static NEXT_QID: Mutex<i32> = Mutex::new(1);

pub const IPC_CREAT: i32 = 0o1000;
pub const IPC_EXCL: i32 = 0o2000;
pub const IPC_RMID: i32 = 0;

pub fn msgget(key: i32, flags: i32) -> Result<i32, MsgError> {
    let mut queues = QUEUES.lock();
    for slot in queues.iter() {
        if let Some(q) = slot {
            if q.key == key {
                return Ok(q.id);
            }
        }
    }
    if flags & IPC_CREAT == 0 {
        return Err(MsgError::NoEnt);
    }
    if flags & IPC_EXCL != 0 {
        for slot in queues.iter() {
            if let Some(q) = slot {
                if q.key == key {
                    return Err(MsgError::Exist);
                }
            }
        }
    }
    let qid = {
        let mut next = NEXT_QID.lock();
        let id = *next;
        *next += 1;
        id
    };
    let owner = crate::proc::current_thread(|t| t.pid.as_u32()).unwrap_or(0);
    for slot in queues.iter_mut() {
        if slot.is_none() {
            *slot = Some(MsgQueue::new(qid, key, owner));
            return Ok(qid);
        }
    }
    Err(MsgError::NoSpace)
}

pub fn msgsnd(qid: i32, mtype: i64, data: &[u8]) -> Result<(), MsgError> {
    if data.len() > MAX_MSG_SIZE {
        return Err(MsgError::Inval);
    }
    let mut queues = QUEUES.lock();
    let q = find_queue_mut(&mut queues, qid)?;
    if q.msg_count >= MAX_MSGS || q.current_bytes + data.len() > q.max_bytes {
        return Err(MsgError::Again);
    }
    for slot in q.msgs.iter_mut() {
        if slot.is_none() {
            let mut msg = Message {
                mtype,
                len: data.len(),
                data: [0; MAX_MSG_SIZE],
            };
            msg.data[..data.len()].copy_from_slice(data);
            *slot = Some(msg);
            q.msg_count += 1;
            q.current_bytes += data.len();
            return Ok(());
        }
    }
    Err(MsgError::Again)
}

pub fn msgrcv(qid: i32, mtype: i64, buf: &mut [u8], truncate: bool) -> Result<(i64, usize), MsgError> {
    let mut queues = QUEUES.lock();
    let q = find_queue_mut(&mut queues, qid)?;
    let idx = q
        .msgs
        .iter()
        .position(|m| m.as_ref().map(|msg| msg.mtype == mtype || mtype == 0).unwrap_or(false))
        .ok_or(MsgError::Again)?;
    let msg = q.msgs[idx].take().unwrap();
    q.msg_count -= 1;
    q.current_bytes -= msg.len;
    let copy_len = if truncate || buf.len() >= msg.len {
        msg.len
    } else {
        buf.len()
    };
    buf[..copy_len].copy_from_slice(&msg.data[..copy_len]);
    Ok((msg.mtype, copy_len))
}

pub fn msgctl(qid: i32, cmd: i32) -> Result<(), MsgError> {
    if cmd != IPC_RMID {
        return Err(MsgError::Inval);
    }
    let mut queues = QUEUES.lock();
    for slot in queues.iter_mut() {
        if let Some(q) = slot {
            if q.id == qid {
                *slot = None;
                return Ok(());
            }
        }
    }
    Err(MsgError::Inval)
}

fn find_queue_mut<'a>(
    queues: &'a mut [Option<MsgQueue>; MAX_QUEUES],
    qid: i32,
) -> Result<&'a mut MsgQueue, MsgError> {
    for slot in queues.iter_mut() {
        if let Some(q) = slot {
            if q.id == qid {
                return Ok(q);
            }
        }
    }
    Err(MsgError::Inval)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgError {
    Inval,
    NoEnt,
    Exist,
    NoSpace,
    Again,
}

pub fn msg_err_to_errno(err: MsgError) -> crate::syscall::errno::Errno {
    use crate::syscall::errno::Errno;
    match err {
        MsgError::Inval => Errno::EINVAL,
        MsgError::NoEnt => Errno::ENOENT,
        MsgError::Exist => Errno::EEXIST,
        MsgError::NoSpace => Errno::ENOSPC,
        MsgError::Again => Errno::EAGAIN,
    }
}
