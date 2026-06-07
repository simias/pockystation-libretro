//! Allocating fixed size arrays on the heap without unsafe code is pretty hard while the box
//! syntax is unstable. As a workaround I implement a small wrapper around unsafe
//! `into_boxed_slice` shenanigans.

use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeTuple, Serializer};
use std::fmt;
use std::marker::PhantomData;
use std::ops;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BoxArray<T, const N: usize> {
    array: Box<[T; N]>,
}

impl<T, const N: usize> BoxArray<T, N> {
    /// Creates a new instance of BoxArray from the given Vec. Panics if the length of the vector
    /// is not equal to `N`
    pub fn from_vec(v: Vec<T>) -> BoxArray<T, N> {
        assert_eq!(
            v.len(),
            N,
            "Attempted to create a BoxArray from a Vec of the wrong size"
        );

        let boxed_slice = v.into_boxed_slice();

        let ptr = ::std::boxed::Box::into_raw(boxed_slice) as *mut [T; N];
        let array = unsafe { Box::from_raw(ptr) };

        BoxArray { array }
    }
}

impl<T, const N: usize> ops::Deref for BoxArray<T, N> {
    type Target = [T; N];

    fn deref(&self) -> &Self::Target {
        &self.array
    }
}

impl<T, const N: usize> ops::DerefMut for BoxArray<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.array
    }
}

impl<T: Serialize, const N: usize> Serialize for BoxArray<T, N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tup = serializer.serialize_tuple(N)?;

        for v in self.array.iter() {
            tup.serialize_element(v)?;
        }

        tup.end()
    }
}

impl<'de, T, const N: usize> Deserialize<'de> for BoxArray<T, N>
where
    T: Deserialize<'de> + Copy,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BoxArrayVisitor<T, const N: usize> {
            phantom: PhantomData<[T; N]>,
        }

        impl<'de, T, const N: usize> Visitor<'de> for BoxArrayVisitor<T, N>
        where
            T: Deserialize<'de> + Copy,
        {
            type Value = BoxArray<T, N>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "an array of {} elements", N)
            }

            fn visit_seq<S>(self, mut seq: S) -> std::result::Result<BoxArray<T, N>, S::Error>
            where
                S: SeqAccess<'de>,
            {
                let mut v = Vec::with_capacity(N);

                for i in 0..N {
                    match seq.next_element()? {
                        Some(e) => v.push(e),
                        None => return Err(de::Error::invalid_length(i, &self)),
                    }
                }

                Ok(BoxArray::from_vec(v))
            }
        }

        let visitor = BoxArrayVisitor {
            phantom: PhantomData,
        };
        deserializer.deserialize_tuple(N, visitor)
    }
}

#[test]
fn boxarray_serialize() {
    let mut ba: BoxArray<u8, 512> = BoxArray::from_vec(vec![0; 512]);

    for (i, b) in ba.iter_mut().enumerate() {
        let i = i as u8;

        *b = i ^ (i << 1);
    }

    let mut fb = flexbuffers::FlexbufferSerializer::new();

    ba.serialize(&mut fb).unwrap();

    let fbr = flexbuffers::Reader::get_root(fb.view()).unwrap();

    let ba_out = BoxArray::deserialize(fbr).unwrap();

    assert_eq!(ba, ba_out)
}
