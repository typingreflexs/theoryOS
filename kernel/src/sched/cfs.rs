/// Completely Fair Scheduler — weight table and vruntime accounting.

pub const NICE_0_LOAD: u64 = 1024;
pub const SCHED_LATENCY_NS: u64 = 6_000_000; // 6 ms
pub const MIN_GRANULARITY_NS: u64 = 750_000; // 0.75 ms
pub const TARGET_LATENCY_TICKS: u64 = 4;

/// Linux-style nice-to-weight mapping (nice -20 .. 19).
const NICE_WEIGHT: [u64; 40] = [
    88761, 71755, 56483, 46273, 36291, 29154, 23254, 18705, 14949, 11916, 9548, 7620, 6100, 4904,
    3906, 3121, 2501, 1991, 1586, 1277, 1024, 820, 655, 526, 423, 335, 272, 215, 172, 137, 110, 87,
    70, 56, 45, 36, 29, 23, 18, 15,
];

pub fn weight_for_nice(nice: i8) -> u64 {
    let idx = (nice as i32 + 20).clamp(0, 39) as usize;
    NICE_WEIGHT[idx]
}

pub fn calc_delta_fair(delta: u64, weight: u64) -> u64 {
    if weight == NICE_0_LOAD {
        delta
    } else {
        delta.saturating_mul(NICE_0_LOAD) / weight.max(1)
    }
}

pub fn update_vruntime(vruntime: u64, delta_exec: u64, weight: u64) -> u64 {
    vruntime.saturating_add(calc_delta_fair(delta_exec, weight))
}

pub fn place_entity(min_vruntime: u64, vruntime: u64) -> u64 {
    if vruntime >= min_vruntime {
        vruntime
    } else {
        min_vruntime
    }
}
