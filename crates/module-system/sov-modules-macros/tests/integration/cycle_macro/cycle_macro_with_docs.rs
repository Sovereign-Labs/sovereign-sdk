#![deny(missing_docs)]
#![allow(dead_code)]
//! Crate documentation

use sov_modules_macros::cycle_tracker;

/// Some documentation for function
#[cycle_tracker]
pub fn _function_without_params() {}

/// Here goes the main
pub fn main() {}
