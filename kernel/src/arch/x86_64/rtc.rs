//! CMOS real-time clock driver.
//!
//! Reads wall-clock time from the standard PC RTC chip (Motorola MC146818
//! compatible) at I/O ports 0x70/0x71. Handles BCD encoding, the 12-hour
//! mode quirk, and the "update in progress" race.

use core::sync::atomic::{AtomicI64, AtomicU8, Ordering};

use crate::arch::x86_64::port;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const REG_SECOND: u8 = 0x00;
const REG_MINUTE: u8 = 0x02;
const REG_HOUR: u8 = 0x04;
const REG_DAY: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
const REG_STATUS_A: u8 = 0x0A;
const REG_STATUS_B: u8 = 0x0B;

static CENTURY_REG: AtomicU8 = AtomicU8::new(0);
static BOOT_UNIX_SECS: AtomicI64 = AtomicI64::new(0);

/// Wall-clock time broken down into UTC components.
#[derive(Clone, Copy, Debug, Default)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

unsafe fn read_cmos(reg: u8) -> u8 {
    port::outb(CMOS_ADDR, reg);
    port::inb(CMOS_DATA)
}

fn update_in_progress() -> bool {
    unsafe { read_cmos(REG_STATUS_A) & 0x80 != 0 }
}

/// Read raw CMOS time. Waits for an update window to close so all fields are
/// from the same second.
fn read_raw() -> DateTime {
    while update_in_progress() {
        core::hint::spin_loop();
    }

    let mut last = DateTime::default();
    let century_reg = CENTURY_REG.load(Ordering::Relaxed);
    loop {
        while update_in_progress() {
            core::hint::spin_loop();
        }
        let (sec, min, hr, day, mon, yr, century) = unsafe {
            (
                read_cmos(REG_SECOND),
                read_cmos(REG_MINUTE),
                read_cmos(REG_HOUR),
                read_cmos(REG_DAY),
                read_cmos(REG_MONTH),
                read_cmos(REG_YEAR),
                if century_reg != 0 {
                    read_cmos(century_reg)
                } else {
                    0
                },
            )
        };
        let status_b = unsafe { read_cmos(REG_STATUS_B) };
        let bcd = status_b & 0x04 == 0;
        let twelve_hour = status_b & 0x02 == 0;

        let sec = decode(sec, bcd);
        let min = decode(min, bcd);
        let hr_raw = hr;
        let hr_value = decode(hr_raw & 0x7F, bcd);
        let hour = if twelve_hour && hr_raw & 0x80 != 0 && hr_value != 12 {
            hr_value + 12
        } else if twelve_hour && hr_value == 12 && hr_raw & 0x80 == 0 {
            0
        } else {
            hr_value
        };
        let day = decode(day, bcd);
        let mon = decode(mon, bcd);
        let yr = decode(yr, bcd);
        let cent = if century != 0 { decode(century, bcd) } else { 20 };

        let dt = DateTime {
            year: cent as u16 * 100 + yr as u16,
            month: mon,
            day,
            hour,
            minute: min,
            second: sec,
        };

        if dt.year == last.year
            && dt.month == last.month
            && dt.day == last.day
            && dt.hour == last.hour
            && dt.minute == last.minute
            && dt.second == last.second
        {
            return dt;
        }
        last = dt;
    }
}

fn decode(value: u8, bcd: bool) -> u8 {
    if bcd {
        ((value >> 4) * 10) + (value & 0x0F)
    } else {
        value
    }
}

/// Convert a [`DateTime`] (UTC, proleptic Gregorian) to Unix seconds.
pub fn to_unix(dt: DateTime) -> i64 {
    let year = dt.year as i64;
    let month = dt.month as i64;
    let day = dt.day as i64;
    // Howard Hinnant's days_from_civil algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    days * 86400
        + dt.hour as i64 * 3600
        + dt.minute as i64 * 60
        + dt.second as i64
}

/// Initialize the RTC by stamping a baseline. After this, `unix_now` returns
/// monotonic wall-clock seconds derived from the boot RTC reading plus the
/// monotonic timer.
pub fn init() {
    set_century_register();
    let dt = read_raw();
    let secs = to_unix(dt);
    BOOT_UNIX_SECS.store(secs, Ordering::Relaxed);
    crate::console::Console::println(&alloc::format!(
        "[rtc] {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        dt.year,
        dt.month,
        dt.day,
        dt.hour,
        dt.minute,
        dt.second
    ));
}

fn set_century_register() {
    // The century register's offset is named in the FADT. Default to 0x32
    // if the FADT didn't tell us — that's the IBM PC default.
    let reg = crate::acpi::context()
        .and_then(|ctx| ctx.fadt())
        .and_then(|f| f.century_register())
        .unwrap_or(0);
    CENTURY_REG.store(reg, Ordering::Relaxed);
}

/// Current wall-clock Unix time, derived from the RTC snapshot at boot plus
/// the monotonic timer.
pub fn unix_now() -> i64 {
    let base = BOOT_UNIX_SECS.load(Ordering::Relaxed);
    let elapsed_ns = crate::sched::timer::monotonic_ns();
    base + (elapsed_ns / 1_000_000_000) as i64
}

/// Read the current RTC date/time directly (slower, ~one I/O round-trip).
pub fn now() -> DateTime {
    read_raw()
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM:SS`.
pub fn format_unix(secs: i64) -> alloc::string::String {
    let dt = from_unix(secs);
    alloc::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year,
        dt.month,
        dt.day,
        dt.hour,
        dt.minute,
        dt.second
    )
}

fn from_unix(secs: i64) -> DateTime {
    let days = secs.div_euclid(86400);
    let time = secs.rem_euclid(86400);
    let hour = (time / 3600) as u8;
    let minute = ((time / 60) % 60) as u8;
    let second = (time % 60) as u8;

    // days_to_civil
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    DateTime {
        year: year as u16,
        month: m as u8,
        day: d as u8,
        hour,
        minute,
        second,
    }
}
