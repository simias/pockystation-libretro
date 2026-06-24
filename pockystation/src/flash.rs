use crate::box_array::BoxArray;

use super::Addressable;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Flash {
    data: BoxArray<u8, FLASH_SIZE>,
    /// When true the BIOS is mirrored at address 0. Set on reset so
    /// that the reset vector starts executing from the BIOS.
    bios_at_0: bool,
    /// Physical bank enable bits.
    phys_bank_en: u16,
    /// Physical-to-virtual bank mapping.
    phys_to_virt_bank: [u8; 16],
    /// Virtual-to-physical bank mapping. None if the virtual bank is
    /// not mapped.
    virt_to_phys_bank: [Option<u8>; 16],
    f_wait1: u8,
    f_wait2: u8,
}

impl Flash {
    pub fn new(flash: &[u8]) -> Option<Flash> {
        if flash.len() != FLASH_SIZE {
            return None;
        }

        Some(Flash {
            data: BoxArray::from_vec(flash.to_vec()),
            bios_at_0: true,
            phys_bank_en: 0,
            phys_to_virt_bank: [0; 16],
            virt_to_phys_bank: [None; 16],
            f_wait1: 0,
            f_wait2: 0,
        })
    }

    pub fn reset(&mut self) {
        self.bios_at_0 = true;
    }

    pub fn load_config<A: Addressable>(&self, offset: u32) -> u32 {
        match offset {
            // The BIOS expects bit 0 to be set, otherwise it gets stuck in a strange loop waiting
            // for R0 to become 1 (but it doesn't actually load anything in R0 in the loop, so I
            // don't understand how it's ever supposed to exit it). This loop is at offset 0x2e16
            // and 0x2e18 in the BIOS.
            //
            // In my tests on the real hardware this register seems to read 1 all the time,
            // regardless of what I write at the address (tested with 0, 1, 2, 3, and 0xff).
            //
            // I presume that this reads "1" when the RAM has been remapped at 0 and there's no way
            // to undo this without resetting the device.
            0x00 => 1,
            // XXX figure out what this register does exactly, No$
            // calls it "F_STAT".
            0x04 => 0,
            // Seems to always read 4
            0x0c => 4 as u32,
            // Seems to always read 4
            0x10 => 4 as u32,
            0x08 => self.phys_bank_en as u32,
            0x100..=0x13c => {
                let phys_bank = (offset & 0x3f) >> 2;

                self.phys_to_virt_bank[phys_bank as usize] as u32
            }
            // F_SN
            //
            // This is apparently unique for every PocketStation, this is the value I dumped on my
            // own.
            //
            // No$ says that MGS uses this value as "copy protection", refusing to run if the value
            // differs from the original save.
            //
            // No$ also says that "the two LO/HI registers must be read by separate 16bit LDRH
            // opcodes (not by a single 32bit LDR opcode)", not sure what's that about, the values
            // below were read using the COM "read memory" commands. Maybe it uses 8bit reads under
            // the hood so it works on a 16-bit restricted bus? Needs to check.
            //
            // The BIOS dos read both these registers with LDRH, so maybe those high 16 bits are
            // garbage?
            0x300 => 0x472e_e203,
            // F_CAL.
            0x308 => 0x1d,
            0x310 => 0x10,
            0x37c => 0x5400_000,
            _ => panic!("Unhandled flash config register {:x}", offset),
        }
    }

    pub fn store_config<A: Addressable>(&mut self, offset: u32, val: u32) {
        match offset {
            0x00 => self.set_f_ctrl(val),
            0x08 => {
                self.phys_bank_en = val as u16;
                self.rebuild_virt_mapping();
            }
            0x0c => self.f_wait1 = val as u8,
            // It seems that writing 0x41 in this register might remap the flash containing the
            // serial and calibration values, allowing writes to it.
            0x10 => self.f_wait2 = val as u8,
            0x100..=0x13c => {
                let phys_bank = (offset & 0x3f) >> 2;
                let virt_bank = val & 0xf;

                self.phys_to_virt_bank[phys_bank as usize] = virt_bank as u8;

                self.rebuild_virt_mapping();
            }
            _ => panic!("Unhandled flash config register {:x}", offset),
        }
    }

    pub fn load_raw<A: Addressable>(&self, offset: u32) -> u32 {
        let offset = offset as usize;

        // Flash only supports 16 and 32bit acccess
        if A::size() == 1 {
            panic!("Unsupported 8bit FLASH read");
        }

        let mut r = 0;

        for i in 0..A::size() as usize {
            r |= (self.data[offset + i] as u32) << (8 * i)
        }

        r
    }

    pub fn store_raw<A: Addressable>(&mut self, offset: u32, val: u32) {
        let offset = offset as usize;

        for i in 0..A::size() as usize {
            self.data[offset + i] = (val >> (i * 8)) as u8;
        }
    }

    pub fn load_virtual<A: Addressable>(&self, offset: u32) -> u32 {
        // Resolve the physical bank (each bank is 8KB)
        let virt_bank = offset >> 13;
        let bank_off = offset & 0x1fff;

        match self.virt_to_phys_bank[virt_bank as usize] {
            Some(p) => {
                let phys = ((p as u32) << 13) | bank_off;

                self.load_raw::<A>(phys)
            }
            None => panic!("read from unmapped virtual bank {}", virt_bank),
        }
    }

    pub fn bios_at_0(&self) -> bool {
        self.bios_at_0
    }

    pub fn data(&self) -> &[u8; FLASH_SIZE] {
        &self.data
    }

    pub fn set_data(&mut self, mut data: Vec<u8>) {
        data.resize(FLASH_SIZE, 0xbb);
        self.data = BoxArray::from_vec(data);
    }

    fn set_f_ctrl(&mut self, val: u32) {
        // It doesn't appear to be possible to undo this change without resetting to test
        if val == 0x03 {
            self.bios_at_0 = false;
        } else {
            info!("F_CTRL {val:x}")
        }
    }

    fn rebuild_virt_mapping(&mut self) {
        // XXX this is mostly guesswork, I don't know exactly what
        // happens when two different physical banks are mapped to the
        // same virtual one, I don't know how the enable bits are
        // handled exactly.

        self.virt_to_phys_bank = [None; 16];

        for (p, &v) in self.phys_to_virt_bank.iter().enumerate() {
            // Check if the bank is enabled
            if (self.phys_bank_en & (1u16 << p)) != 0 {
                let vbank = &mut self.virt_to_phys_bank[v as usize];

                match *vbank {
                    None => *vbank = Some(p as u8),
                    Some(other) => {
                        panic!("Virtual bank {} is mapped twice: {} and {}", v, other, p)
                    }
                }
            }
        }
    }
}

/// FLASH size in bytes
pub const FLASH_SIZE: usize = 128 * 1024;
