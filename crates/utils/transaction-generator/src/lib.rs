//! This crate defines interfaces for generating Sovereign SDK `CallMessage`s in a standard
//! way, and uses those interfaces to provide transaction generation for the most common cases.

#![deny(missing_docs)]
pub mod interface;

pub mod state;

pub mod generators;

pub use interface::*;
pub use state::*;
