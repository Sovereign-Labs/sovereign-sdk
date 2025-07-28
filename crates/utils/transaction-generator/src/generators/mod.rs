//! Implementations of the `CallMessageGenerator` trait.
pub mod bank;

/// A basic call message generator factory that can be used with modules internal to the sovereign sdk
pub mod basic;

pub mod factory;
pub mod macros;
/// Implements `Transaction` generation.
pub mod transaction;
pub mod value_setter;

pub mod access_pattern;
pub mod synthetic_load;
