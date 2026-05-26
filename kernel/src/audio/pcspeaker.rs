//! PC speaker driver — PIT channel 2 + port 0x61.
//!
//! Drives the legacy 8254 PIT to generate a square wave on the PC speaker.
//! Universal across every IBM-PC-compatible bare-metal system since 1981.

use crate::arch::x86_64::port;

const PIT_CMD: u16 = 0x43;
const PIT_CH2: u16 = 0x42;
const SPEAKER_PORT: u16 = 0x61;
const PIT_FREQUENCY: u32 = 1_193_182;

pub fn init() {
    silence();
}

/// Play a tone at `frequency_hz` for `duration_ms`. Blocks the caller.
pub fn tone(frequency_hz: u32, duration_ms: u32) {
    if frequency_hz == 0 {
        return;
    }
    let divisor = (PIT_FREQUENCY / frequency_hz) as u16;
    unsafe {
        port::outb(PIT_CMD, 0xB6);
        port::outb(PIT_CH2, (divisor & 0xFF) as u8);
        port::outb(PIT_CH2, ((divisor >> 8) & 0xFF) as u8);
        let prev = port::inb(SPEAKER_PORT);
        if prev & 0x03 != 0x03 {
            port::outb(SPEAKER_PORT, prev | 0x03);
        }
    }
    spin_delay_ms(duration_ms);
    silence();
}

/// Silence the speaker (clear bits 0 and 1 of port 0x61).
pub fn silence() {
    unsafe {
        let prev = port::inb(SPEAKER_PORT);
        port::outb(SPEAKER_PORT, prev & !0x03);
    }
}

fn spin_delay_ms(ms: u32) {
    let target_ns = crate::sched::timer::monotonic_ns() + (ms as u64) * 1_000_000;
    while crate::sched::timer::monotonic_ns() < target_ns {
        core::hint::spin_loop();
    }
}
