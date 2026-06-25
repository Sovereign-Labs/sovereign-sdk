//! Common types and traits used all throughout the Sovereign SDK.
//!
//! # Height and slot numbering
//!
//! How DA heights, slot numbers, and rollup heights relate, and how to convert between
//! them. Several different "heights" show up when reasoning about where the rollup is.
//! They coincide for a freshly started based rollup but diverge in general, so it is
//! worth keeping them straight.
//!
//! ## DA height
//!
//! The block number of the underlying data-availability layer (e.g. a Celestia or
//! Ethereum block height). It is absolute to the DA chain and is not one of the types in
//! this module — it is the input from which a [`SlotNumber`] is derived.
//!
//! ## [`SlotNumber`] — the rollup's slot number
//!
//! The DA height minus the DA height at the rollup's genesis: one slot per DA block the
//! rollup processes, counting from [`SlotNumber::GENESIS`] (`0`). This is the number the
//! ledger API hands out (`SlotResponse::number`, `BatchResponse::slot_number`).
//!
//! ## [`VisibleSlotNumber`]
//!
//! The [`SlotNumber`] at which a user-space state transition becomes visible. It can lag
//! the true slot number, because blobs may be deferred for several slots before they
//! execute, and it advances by one or more slots whenever a rollup block is produced. It
//! never exceeds the true slot number.
//!
//! ## [`RollupHeight`]
//!
//! The number of logical rollup blocks produced so far (`1, 2, 3, ...`), independent of
//! what happens on the DA layer. For a based rollup it equals the slot number; for a
//! soft-confirming rollup it increments by one per batch sent by the preferred sequencer.
//! An empty DA slot advances the slot number but not the rollup height.
//!
//! The committed chain state always satisfies (all three are `0` at genesis):
//!
//! ```text
//! rollup_height <= visible_slot_number <= true_slot_number
//! ```
//!
//! ## Converting a slot number to a rollup height
//!
//! The ledger API indexes by slot number, whereas rollup *state* is indexed by rollup
//! height, so a slot number taken from a ledger subscription often needs translating.
//!
//! - **Over the REST API** (the usual case): every module state query accepts a
//!   mutually-exclusive `?slot_number=N` or `?rollup_height=N` parameter. Passing
//!   `?slot_number=N` makes the node resolve the slot to its rollup height internally, so
//!   to *read state* as of a slot you do not need the height at all. To read the height
//!   itself, query the chain-state module's current heights as of that slot:
//!
//!   ```text
//!   GET /modules/chain-state/state/current-heights?slot_number=N
//!   ```
//!
//!   The response `value` is `[rollup_height, visible_slot_number]`, so `value[0]` is the
//!   rollup height for slot `N`.
//!
//! - **In-process** (`native` only): use the kernel capability
//!   `KernelWithSlotMapping::true_slot_number_to_rollup_height(slot_number, state)`, which
//!   returns `Option<RollupHeight>` (`None` until the slot has been processed).

mod hex_string;
pub mod safe_vec;
mod slot_numbering;
mod strict_bincode;

pub use hex_string::*;
pub use safe_vec::SafeVec;
pub use slot_numbering::*;
pub use sov_universal_wallet::schema::safe_string::{SafeString, SizedSafeString};
pub use strict_bincode::strict_bincode_deserialize;
