#![no_main]
//! This binary implements the verification logic for the rollup. This is the code that runs inside
//! of the zkvm in order to generate proofs for the rollup.
risc0_zkvm::guest::entry!(main);

pub fn main() {}
