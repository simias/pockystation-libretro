//! LCD controller emulation

use crate::Addressable;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Lcd {
    mode: u8,
    calibration: u8,
    fb: [u32; 32],
}

impl Default for Lcd {
    fn default() -> Self {
        Self::new()
    }
}

impl Lcd {
    pub fn new() -> Lcd {
        Lcd {
            mode: 0,
            calibration: 0,
            fb: [0xaaaa5555; 32],
        }
    }

    pub fn store<A: Addressable>(&mut self, offset: u32, val: u32) {
        if A::size() != 4 {
            panic!("Unhandled {}bit LCD store", A::size() * 8);
        }

        match offset {
            // Writing 0 seems to hang the console? Maybe it's because you can't write to the LCD
            // mem if it's not running, the BIOS init code clears the screen *after* it's been
            // configured.
            //
            // Clearing bit 3 seems to fade the screen off, maybe something refresh-related.
            // Clearing bits 4-5 also switches the screen off with some visual glitches
            0 => self.mode = val as u8,
            4 => self.calibration = val as u8,
            0x100..=0x17c => {
                let i = (offset & 0x7f) as usize;

                self.fb[i / 4] = val;
            }
            _ => panic!("Unhandled LCD register {:x}", offset),
        }
    }

    pub fn load<A: Addressable>(&self, offset: u32) -> u32 {
        if A::size() != 4 {
            panic!("Unhandled {}bit LCD store", A::size() * 8);
        }

        match offset {
            0 => self.mode as u32,
            4 => self.calibration as u32,
            0x100..=0x17c => {
                let i = (offset & 0x7f) as usize;

                self.fb[i / 4]
            }
            _ => panic!("Unhandled LCD register {:x}", offset),
        }
    }

    pub fn framebuffer(&self) -> &[u32; 32] {
        &self.fb
    }

    /// Return true if the screen rotation flag is set
    pub fn rotated(&self) -> bool {
        self.mode & 0x80 != 0
    }
}
