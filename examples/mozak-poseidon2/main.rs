//! The main objective is to benchmark the performance of the
//! poseidon2 ecall in the context of flat hashing tapes.
//! The tapes which we might want to flat hash are public
//! tape, and call tape

#![cfg_attr(target_os = "mozakvm", no_main)]
#![cfg_attr(not(feature = "std"), no_std)]

use core::hint::black_box;
extern crate alloc;
use alloc::vec::Vec;

use mozak_sdk::core::ecall::{ioread_public, poseidon2};

#[allow(clippy::unit_arg)]
fn main() {
    // number of bytes we would hash.
    let n = {
        let mut bytes = [0u8; 4];
        ioread_public(bytes.as_mut_ptr(), bytes.len());
        u32::from_le_bytes(bytes).next_multiple_of(8)
    };

    // using a deterministic vector of bytes since
    // using rng to generate random bytes interferes
    // with benching poseidon2 ecall, which is the primary
    // focus here.
    let v: Vec<u8> = black_box((0..n).map(|i| 0x12).collect());
    let mut hash = [0u8; 32];

    // flat hash
    black_box(poseidon2(v.as_ptr(), v.len(), hash.as_mut_ptr()));
}

mozak_sdk::entry!(main);
