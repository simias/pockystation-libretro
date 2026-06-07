//! PocketStation Audio DAC emulation

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use memory::Addressable;
use MASTER_CLOCK_HZ;

pub struct Dac {
    /// Current output sample. Not sure how many bits are used on the
    /// real hardware, No$ says 8bit but Wikipedia seems to say that
    /// it's 10bit. Won't matter much given how crappy the sound is on
    /// the real hardware anyway...
    sample: i16,
    enabled: bool,
    backend: Box<Backend>,
    /// Master clock divider
    divider: u32,
}

impl Dac {
    pub fn new(backend: Box<Backend>) -> Dac {
        Dac {
            sample: 0,
            enabled: false,
            backend: backend,
            divider: MASTER_CLOCK_DIV,
        }
    }

    pub fn tick(&mut self, mut master_ticks: u32) {

        while master_ticks > 0 {
            if self.divider >= master_ticks {
                self.divider -= master_ticks;

                master_ticks = 0;
            } else {
                master_ticks -= self.divider;

                self.divider = MASTER_CLOCK_DIV;

                // Time to generate a sample
                let sample =
                    if self.enabled {
                        self.sample
                    } else {
                        0
                    };

                self.backend.push_sample(sample);
            }
        }
    }


    pub fn store<A: Addressable>(&mut self, offset: u32, val: u32) {
        if A::size() == 1 {
            // XXX Brightis uses 16bit dac stores, test if it behaves
            // exactly like 32bit
            panic!("Unhandled {}bit DAC store", A::size() * 8);
        }

        match offset {
            0 => self.enabled = (val & 1) != 0,
            4 => self.sample = val as i16,
            _ => panic!("Unhandled DAC register {:x}", offset),
        }
    }

    pub fn load<A: Addressable>(&self, offset: u32) -> u32 {
        if A::size() != 4 {
            panic!("Unhandled {}bit DAC store", A::size() * 8);
        }

        match offset {
            0 => self.enabled as u32,
            4 => self.sample as u16 as u32,
            _ => panic!("Unhandled DAC register {:x}", offset),
        }
    }

    pub fn set_backend(&mut self, backend: Box<Backend>) {
        self.backend = backend
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedDac {
    sample: i16,
    enabled: bool,
    divider: u32,
}

impl Serialize for Dac {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = SerializedDac {
            sample: self.sample,
            enabled: self.enabled,
            divider: self.divider,
        };

        s.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Dac {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = SerializedDac::deserialize(deserializer)?;


        let mut dac = Dac::new(Box::new(DummyBackend));
        dac.sample = s.sample;
        dac.enabled = s.enabled;
        dac.divider = s.divider;

        Ok(dac)
    }
}

pub trait Backend {
    fn push_sample(&mut self, sample: i16);
}

struct DummyBackend;

impl Backend for DummyBackend {
    fn push_sample(&mut self, _: i16) {
    }
}

/// Technically the audio frequency could reach MASTER_CLOCK_HZ (if
/// the CPU keeps writing a new value at every cycle at max frequency)
/// but it would be pointless to have a 4MHz audio sample rate, so I
/// divide it to a more reasonable value. A value of 90 results in a
/// sample rate of around 44.4kHz, a little more than CD
/// quality. Should be more than enough.
pub const MASTER_CLOCK_DIV: u32 = 90;

/// Audio sample rate.
pub const SAMPLE_RATE_HZ: u32 = MASTER_CLOCK_HZ / MASTER_CLOCK_DIV;
