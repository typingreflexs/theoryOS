use spin::Mutex;

use crate::arch::x86_64::smp;
use crate::proc::id::Tid;
use crate::proc::table;
use crate::proc::thread::{Thread, ThreadState};
use crate::sched::cfs;
use crate::sched::rbtree::{RbNodeId, RedBlackTree};

pub const MAX_CPUS: usize = 256;

pub struct CfsRunQueue {
    pub cpu: u32,
    pub tree: RedBlackTree,
    pub nr_running: u32,
    pub min_vruntime: u64,
    pub curr: Tid,
    pub idle: Tid,
}

impl CfsRunQueue {
    pub const fn new(cpu: u32) -> Self {
        Self {
            cpu,
            tree: RedBlackTree::new(),
            nr_running: 0,
            min_vruntime: 0,
            curr: Tid::INVALID,
            idle: Tid::INVALID,
        }
    }

    pub fn init(&mut self) {
        self.tree.init();
    }

    pub fn enqueue(&mut self, tid: Tid) {
        if let Some(vruntime) = table::with_thread_mut(tid, |t| {
            t.vruntime = cfs::place_entity(self.min_vruntime, t.effective_vruntime());
            let key = t.effective_vruntime();
            let node = self.tree.insert(key, tid);
            t.rb_node = node.unwrap_or(RbNodeId::INVALID);
            t.on_rq = true;
            t.state = ThreadState::Runnable;
            t.cpu = self.cpu;
            key
        }) {
            let _ = vruntime;
            self.nr_running += 1;
            if let Some(min_key) = self.tree.peek_min_key() {
                self.min_vruntime = min_key;
            }
        }
    }

    pub fn dequeue(&mut self, tid: Tid) {
        table::with_thread_mut(tid, |t| {
            if t.on_rq && t.rb_node.is_valid() {
                self.tree.remove(t.rb_node);
                t.rb_node = RbNodeId::INVALID;
                t.on_rq = false;
                self.nr_running = self.nr_running.saturating_sub(1);
            }
        });
    }

    pub fn pick_next(&mut self) -> Tid {
        if let Some((_, tid, key)) = self.tree.min() {
            self.tree.remove(
                table::with_thread(tid, |t| t.rb_node).unwrap_or(RbNodeId::INVALID),
            );
            table::with_thread_mut(tid, |t| {
                t.on_rq = false;
                t.rb_node = RbNodeId::INVALID;
                t.state = ThreadState::Running;
            });
            self.nr_running = self.nr_running.saturating_sub(1);
            self.min_vruntime = key;
            return tid;
        }
        self.idle
    }

    /// Work stealing: take the leftmost ( fairest ) thread from another CPU's queue.
    pub fn steal_from(&mut self, victim: &mut CfsRunQueue) -> Option<Tid> {
        if victim.cpu == self.cpu || victim.tree.is_empty() {
            return None;
        }
        let (_, tid, key) = victim.tree.min()?;
        let node = table::with_thread(tid, |t| t.rb_node).unwrap_or(RbNodeId::INVALID);
        victim.tree.remove(node);
        victim.nr_running = victim.nr_running.saturating_sub(1);
        table::with_thread_mut(tid, |t| {
            t.on_rq = false;
            t.rb_node = RbNodeId::INVALID;
            t.cpu = self.cpu;
            t.vruntime = cfs::place_entity(self.min_vruntime, t.effective_vruntime());
            let new_node = self.tree.insert(t.effective_vruntime(), tid);
            t.rb_node = new_node.unwrap_or(RbNodeId::INVALID);
            t.on_rq = true;
        });
        self.nr_running += 1;
        self.min_vruntime = self.tree.peek_min_key().unwrap_or(key);
        Some(tid)
    }
}

static RUNQUEUES: Mutex<[CfsRunQueue; MAX_CPUS]> =
    Mutex::new([const { CfsRunQueue::new(0) }; MAX_CPUS]);

pub fn init_cpu(cpu: u32, idle_tid: Tid) {
    let mut queues = RUNQUEUES.lock();
    queues[cpu as usize] = CfsRunQueue::new(cpu);
    queues[cpu as usize].init();
    queues[cpu as usize].idle = idle_tid;
}

pub fn with_runqueue<F, R>(cpu: u32, f: F) -> R
where
    F: FnOnce(&mut CfsRunQueue) -> R,
{
    f(&mut RUNQUEUES.lock()[cpu as usize])
}

pub fn enqueue_thread(tid: Tid, cpu: u32) {
    with_runqueue(cpu, |rq| rq.enqueue(tid));
}

pub fn pick_next_thread(cpu: u32) -> Tid {
    with_runqueue(cpu, |rq| {
        if rq.nr_running == 0 {
            if let Some(tid) = try_steal(cpu) {
                return tid;
            }
        }
        rq.pick_next()
    })
}

fn try_steal(cpu: u32) -> Option<Tid> {
    let cpu_count = smp::cpu_count();
    for other in 0..cpu_count {
        if other == cpu {
            continue;
        }
        let mut guard = RUNQUEUES.lock();
        let cpu_idx = cpu as usize;
        let other_idx = other as usize;
        if guard[other_idx].nr_running <= 1 {
            continue;
        }
        let stolen = if cpu_idx < other_idx {
            let (lo, hi) = guard.split_at_mut(other_idx);
            lo[cpu_idx].steal_from(&mut hi[0])
        } else if cpu_idx > other_idx {
            let (lo, hi) = guard.split_at_mut(cpu_idx);
            hi[0].steal_from(&mut lo[other_idx])
        } else {
            None
        };
        if stolen.is_some() {
            return stolen;
        }
    }
    None
}

pub fn update_current_runtime(cpu: u32, delta_ns: u64) {
    with_runqueue(cpu, |rq| {
        let curr = rq.curr;
        if !curr.is_valid() {
            return;
        }
        table::with_thread_mut(curr, |t| {
            t.sum_exec_runtime = t.sum_exec_runtime.saturating_add(delta_ns);
            t.vruntime = cfs::update_vruntime(t.vruntime, delta_ns, t.weight);
        });
    });
}

pub fn set_current(cpu: u32, tid: Tid) {
    RUNQUEUES.lock()[cpu as usize].curr = tid;
}

pub fn current_on_cpu(cpu: u32) -> Tid {
    RUNQUEUES.lock()[cpu as usize].curr
}

pub fn runnable_count(cpu: u32) -> u32 {
    RUNQUEUES.lock()[cpu as usize].nr_running
}
