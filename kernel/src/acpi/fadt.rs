use crate::arch::memory::{phys_to_virt, PhysAddr, VirtAddr};

#[repr(C, packed)]
struct FadtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
    firmware_ctrl: u32,
    dsdt: u32,
    reserved: u8,
    preferred_pm_profile: u8,
    sci_int: u16,
    smi_cmd: u32,
    acpi_enable: u8,
    acpi_disable: u8,
    s4bios_req: u8,
    pstate_cnt: u8,
    pm1a_evt_blk: u32,
    pm1b_evt_blk: u32,
    pm1a_cnt_blk: u32,
    pm1b_cnt_blk: u32,
    pm2_cnt_blk: u32,
    pm_tmr_blk: u32,
    gpe0_blk: u32,
    gpe1_blk: u32,
    pm1_evt_len: u8,
    pm1_cnt_len: u8,
    pm2_cnt_len: u8,
    pm_tmr_len: u8,
    gpe0_blk_len: u8,
    gpe1_blk_len: u8,
    gpe1_base: u8,
    cst_cnt: u8,
    p_lvl2_lat: u16,
    p_lvl3_lat: u16,
    flush_size: u16,
    flush_stride: u16,
    duty_offset: u8,
    duty_width: u8,
    day_alrm: u8,
    mon_alrm: u8,
    century: u8,
    iapc_boot_arch: u16,
    reserved2: u8,
    flags: u32,
    reset_reg: GenericAddress,
    reset_value: u8,
    arm_boot_arch: u16,
    minor_version: u8,
    x_dsdt: u64,
    x_pm1a_evt_blk: GenericAddress,
    x_pm1b_evt_blk: GenericAddress,
    x_pm1a_cnt_blk: GenericAddress,
    x_pm1b_cnt_blk: GenericAddress,
    x_pm2_cnt_blk: GenericAddress,
    x_pm_tmr_blk: GenericAddress,
    x_gpe0_blk: GenericAddress,
    x_gpe1_blk: GenericAddress,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GenericAddress {
    address_space: u8,
    bit_width: u8,
    bit_offset: u8,
    access_size: u8,
    address: u64,
}

#[derive(Debug)]
pub struct Fadt {
    hhdm: u64,
    pm_timer_block: Option<PhysAddr>,
    pm_timer_length: u8,
    dsdt_phys: Option<PhysAddr>,
    flags: u32,
}

impl Fadt {
    pub fn parse(hhdm: u64, table: VirtAddr) -> Self {
        unsafe {
            let header = &*(table.as_ptr::<FadtHeader>());
            let pm_timer = if header.pm_tmr_blk != 0 {
                Some(PhysAddr::new(header.pm_tmr_blk as u64))
            } else if header.x_pm_tmr_blk.address != 0 {
                Some(PhysAddr::new(header.x_pm_tmr_blk.address))
            } else {
                None
            };

            let dsdt_phys = if header.x_dsdt != 0 {
                Some(PhysAddr::new(header.x_dsdt))
            } else if header.dsdt != 0 {
                Some(PhysAddr::new(header.dsdt as u64))
            } else {
                None
            };

            Self {
                hhdm,
                pm_timer_block: pm_timer,
                pm_timer_length: header.pm_tmr_len,
                dsdt_phys,
                flags: header.flags,
            }
        }
    }

    pub fn dsdt_address(&self) -> Option<PhysAddr> {
        self.dsdt_phys
    }

    pub fn timer_frequency_hz(&self) -> Option<u32> {
        // ACPI PM timer runs at 3.579545 MHz.
        if self.pm_timer_block.is_some() {
            Some(3_579_545)
        } else {
            None
        }
    }

    pub fn reset_supported(&self) -> bool {
        self.flags & (1 << 10) != 0
    }

    pub fn read_pm_timer(&self) -> Option<u32> {
        let phys = self.pm_timer_block?;
        let virt = phys_to_virt(self.hhdm, phys);
        unsafe { Some(virt.as_ptr::<u32>().read_volatile() & 0x00FF_FFFF) }
    }
}
