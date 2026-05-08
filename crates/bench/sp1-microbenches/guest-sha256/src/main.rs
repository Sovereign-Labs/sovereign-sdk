#![no_main]

sp1_zkvm::entrypoint!(main);

use sha2::{Digest, Sha256};

pub fn main() {
    let byte_len: u32 = sp1_zkvm::io::read();
    let iterations: u32 = sp1_zkvm::io::read();

    // Asymmetric pattern: avoids endianness ambiguity and prevents the optimizer
    // from constant-folding to a known digest.
    let buf: Vec<u8> = (0..byte_len)
        .map(|i| (i as u8).wrapping_mul(0xAB))
        .collect();

    println!("cycle-tracker-report-start: hash_loop");
    let mut last = [0u8; 32];
    for _ in 0..iterations {
        last = Sha256::digest(&buf).into();
    }
    println!("cycle-tracker-report-end: hash_loop");

    // Commit so the optimizer cannot elide the loop.
    sp1_zkvm::io::commit(&last);
}
