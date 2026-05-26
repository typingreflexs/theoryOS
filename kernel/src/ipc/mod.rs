//! Inter-process communication — pipes, FIFOs, Unix sockets, mq, shm, futex.

pub mod fifo;
pub mod futex;
pub mod mq;
pub mod pipe;
pub mod shm;
pub mod unix;
pub mod wait;

pub use fifo::{decode_rdev, encode_rdev, lookup as fifo_lookup, mkfifo, register as fifo_register};
pub use futex::{futex, futex_err_to_errno, FUTEX_PRIVATE_FLAG, FUTEX_WAIT, FUTEX_WAKE};
pub use mq::{msg_err_to_errno, msgctl, msgget, msgrcv, msgsnd, IPC_CREAT as MQ_IPC_CREAT, IPC_EXCL as MQ_IPC_EXCL, IPC_RMID as MQ_IPC_RMID};
pub use pipe::{pipe_err_to_errno, Pipe, PipeError, PipeId, PIPE_BUF};
pub use shm::{shm_err_to_errno, shmat, shmctl, shmdt, shmget, IPC_CREAT as SHM_IPC_CREAT, IPC_EXCL as SHM_IPC_EXCL, IPC_RMID as SHM_IPC_RMID, SHM_RDONLY};
pub use unix::{unix_err_to_errno, UnixError, UnixSocketId, UnixType};

use crate::console::Console;

pub fn init() {
    Console::println("[ipc] pipes, FIFOs, unix sockets, mq, shm, futex ready");
}
