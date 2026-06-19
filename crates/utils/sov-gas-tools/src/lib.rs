//! Tools for calibrating and pricing gas constants from microbenchmarks.
//!
//! - [`fit`] — ordinary-least-squares fit of the cost model
//!   `cost = bias + per_byte * size`, shared by the SP1 (prover-gas) and native
//!   (wall-clock) microbenches and reusable by downstream rollup benches.
//! - [`report`] — helpers for the native wall-clock microbenches: reading
//!   criterion estimates, the `1 gas = 0.01 ns` conversion, and printing
//!   suggested `constants.toml` values.
//!
//! Host-only post-processing — never compiled into a zkVM guest, so the workspace
//! `clippy::float_arithmetic` deny (which guards against native/zkVM divergence)
//! doesn't apply.
#![allow(clippy::float_arithmetic)]

pub mod fit;
pub mod report;
