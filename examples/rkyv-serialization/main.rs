#![no_main]
#![no_std]

extern crate alloc;
mod core_logic;

use alloc::string::ToString;
use alloc::vec;

use rkyv::Deserialize;

use crate::core_logic::Test;

pub fn main() {
    let value = Test {
        int: 42,
        string: "Mozak Rocks!!".to_string(),
        option: Some(vec![1, 2, 3, 4]),
    };

    // Serializing is as easy as a single function call
    let bytes = rkyv::to_bytes::<_, 256>(&value).unwrap();

    // Or you can use the unsafe API for maximum performance
    let archived = unsafe { rkyv::archived_root::<Test>(&bytes[..]) };
    assert_eq!(archived, &value);

    // And you can always deserialize back to the original type
    let deserialized: Test = archived.deserialize(&mut rkyv::Infallible).unwrap();
    assert_eq!(deserialized, value);
    let bytes = rkyv::to_bytes::<_, 256>(&deserialized).unwrap();
    guest::env::write(&bytes);
}

guest::entry!(main);
