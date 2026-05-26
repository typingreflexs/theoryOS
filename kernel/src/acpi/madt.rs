use crate::arch::memory::PhysAddr;

#[repr(C, packed)]
struct MadtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
    local_apic_address: u32,
    flags: u32,
}

const TYPE_LOCAL_APIC: u8 = 0;
const TYPE_IO_APIC: u8 = 1;
const TYPE_INTERRUPT_OVERRIDE: u8 = 2;
const TYPE_LOCAL_APIC_NMI: u8 = 4;
const TYPE_LOCAL_APIC_ADDRESS_OVERRIDE: u8 = 5;
const TYPE_IO_SAPIC: u8 = 6;
const TYPE_LOCAL_SAPIC: u8 = 7;
const TYPE_PLATFORM_INTERRUPT: u8 = 8;
const MAX_MADT_LEN: u32 = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct LocalApicEntry {
    pub processor_id: u8,
    pub apic_id: u8,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct IoApicEntry {
    pub io_apic_id: u8,
    pub address: PhysAddr,
    pub global_system_interrupt_base: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct InterruptOverrideEntry {
    pub bus: u8,
    pub irq: u8,
    pub global_system_interrupt: u32,
    pub flags: u16,
}

#[derive(Debug)]
pub struct Madt {
    local_apic_address: PhysAddr,
    local_apics: [LocalApicEntry; 256],
    local_apic_count: usize,
    io_apics: [IoApicEntry; 8],
    io_apic_count: usize,
    interrupt_overrides: [InterruptOverrideEntry; 64],
    interrupt_override_count: usize,
}

impl Madt {
    pub fn qemu_default(bsp_lapic_id: u32) -> Self {
        let mut madt = Self {
            local_apic_address: PhysAddr::new(0xFEE0_0000),
            local_apics: [LocalApicEntry {
                processor_id: 0,
                apic_id: 0,
                enabled: false,
            }; 256],
            local_apic_count: 1,
            io_apics: [IoApicEntry {
                io_apic_id: 0,
                address: PhysAddr::new(0),
                global_system_interrupt_base: 0,
            }; 8],
            io_apic_count: 1,
            interrupt_overrides: [InterruptOverrideEntry {
                bus: 0,
                irq: 0,
                global_system_interrupt: 0,
                flags: 0,
            }; 64],
            interrupt_override_count: 0,
        };
        madt.local_apics[0] = LocalApicEntry {
            processor_id: 0,
            apic_id: bsp_lapic_id as u8,
            enabled: true,
        };
        madt.io_apics[0] = IoApicEntry {
            io_apic_id: 0,
            address: PhysAddr::new(0xFEC0_0000),
            global_system_interrupt_base: 0,
        };
        madt
    }

    pub fn parse(table: crate::arch::memory::VirtAddr) -> Self {
        let mut madt = Self {
            local_apic_address: PhysAddr::new(0xFEE0_0000),
            local_apics: [LocalApicEntry {
                processor_id: 0,
                apic_id: 0,
                enabled: false,
            }; 256],
            local_apic_count: 0,
            io_apics: [IoApicEntry {
                io_apic_id: 0,
                address: PhysAddr::new(0),
                global_system_interrupt_base: 0,
            }; 8],
            io_apic_count: 0,
            interrupt_overrides: [InterruptOverrideEntry {
                bus: 0,
                irq: 0,
                global_system_interrupt: 0,
                flags: 0,
            }; 64],
            interrupt_override_count: 0,
        };

        unsafe {
            let header = &*(table.as_ptr::<MadtHeader>());
            if header.length < core::mem::size_of::<MadtHeader>() as u32
                || header.length > MAX_MADT_LEN
            {
                return madt;
            }
            madt.local_apic_address = PhysAddr::new(header.local_apic_address as u64);

            let mut offset = core::mem::size_of::<MadtHeader>();
            let end = header.length as usize;
            while offset + 2 <= end {
                let entry_ptr = table.as_ptr::<u8>().add(offset);
                let entry_type = *entry_ptr;
                let entry_length = *entry_ptr.add(1) as usize;
                if entry_length < 2 || offset + entry_length > end {
                    break;
                }

                match entry_type {
                    TYPE_LOCAL_APIC if entry_length >= 8 => {
                        let processor_id = *entry_ptr.add(2);
                        let apic_id = *entry_ptr.add(3);
                        let flags = *entry_ptr.add(4);
                        if madt.local_apic_count < madt.local_apics.len() {
                            madt.local_apics[madt.local_apic_count] = LocalApicEntry {
                                processor_id,
                                apic_id,
                                enabled: flags & 1 != 0,
                            };
                            madt.local_apic_count += 1;
                        }
                    }
                    TYPE_IO_APIC if entry_length >= 12 => {
                        let io_apic_id = *entry_ptr.add(2);
                        let address = u32::from_le_bytes([
                            *entry_ptr.add(4),
                            *entry_ptr.add(5),
                            *entry_ptr.add(6),
                            *entry_ptr.add(7),
                        ]);
                        let gsi_base = u32::from_le_bytes([
                            *entry_ptr.add(8),
                            *entry_ptr.add(9),
                            *entry_ptr.add(10),
                            *entry_ptr.add(11),
                        ]);
                        if madt.io_apic_count < madt.io_apics.len() {
                            madt.io_apics[madt.io_apic_count] = IoApicEntry {
                                io_apic_id,
                                address: PhysAddr::new(address as u64),
                                global_system_interrupt_base: gsi_base,
                            };
                            madt.io_apic_count += 1;
                        }
                    }
                    TYPE_INTERRUPT_OVERRIDE if entry_length >= 10 => {
                        let bus = *entry_ptr.add(2);
                        let irq = *entry_ptr.add(3);
                        let gsi = u32::from_le_bytes([
                            *entry_ptr.add(4),
                            *entry_ptr.add(5),
                            *entry_ptr.add(6),
                            *entry_ptr.add(7),
                        ]);
                        let flags = u16::from_le_bytes([*entry_ptr.add(8), *entry_ptr.add(9)]);
                        if madt.interrupt_override_count < madt.interrupt_overrides.len() {
                            madt.interrupt_overrides[madt.interrupt_override_count] =
                                InterruptOverrideEntry {
                                    bus,
                                    irq,
                                    global_system_interrupt: gsi,
                                    flags,
                                };
                            madt.interrupt_override_count += 1;
                        }
                    }
                    TYPE_LOCAL_APIC_ADDRESS_OVERRIDE if entry_length >= 12 => {
                        let address = u64::from_le_bytes([
                            *entry_ptr.add(4),
                            *entry_ptr.add(5),
                            *entry_ptr.add(6),
                            *entry_ptr.add(7),
                            *entry_ptr.add(8),
                            *entry_ptr.add(9),
                            *entry_ptr.add(10),
                            *entry_ptr.add(11),
                        ]);
                        madt.local_apic_address = PhysAddr::new(address);
                    }
                    _ => {}
                }

                offset += entry_length;
            }
        }

        madt
    }

    pub fn local_apic_address(&self) -> PhysAddr {
        self.local_apic_address
    }

    pub fn local_apics(&self) -> &[LocalApicEntry] {
        &self.local_apics[..self.local_apic_count]
    }

    pub fn io_apics(&self) -> &[IoApicEntry] {
        &self.io_apics[..self.io_apic_count]
    }

    pub fn interrupt_overrides(&self) -> &[InterruptOverrideEntry] {
        &self.interrupt_overrides[..self.interrupt_override_count]
    }
}
