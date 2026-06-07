use rustc_serialize::{Decodable, Encodable, Decoder, Encoder};
use crate::box_array::BoxArray;

use super::Addressable;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Ram {
    data: BoxArray<u8, RAM_SIZE>,
}

impl Ram {
    pub fn new() -> Ram {
        Ram {
            data: BoxArray::from_vec(vec![0xca; RAM_SIZE]),
        }
    }

    pub fn load<A: Addressable>(&self, offset: u32) -> u32 {
        let offset = offset as usize;

        let mut r = 0;

        for i in 0..A::size() as usize {
            r |= (self.data[offset + i] as u32) << (8 * i)
        }

        r
    }

    pub fn store<A: Addressable>(&mut self, offset: u32, val: u32) {
        let offset = offset as usize;

        for i in 0..A::size() as usize {
            self.data[offset + i] = (val >> (i * 8)) as u8;
        }
    }
}

/// RAM size in bytes
const RAM_SIZE: usize = 2 * 1024;
