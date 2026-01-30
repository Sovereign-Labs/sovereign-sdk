//! Common types shared between state and modules

mod address;

mod module_id;

use borsh::{BorshDeserialize, BorshSerialize};
pub use module_id::{ModuleId, ModuleIdBech32};

mod amount;
mod crypto;
mod error;
mod module_utils;

pub use address::*;
pub use amount::*;
pub use crypto::*;
pub use error::*;
pub use module_utils::*;
use serde::{Deserialize, Serialize};
pub use sov_state::jmt::Version;

/// The type of sequencer that published a blob.
#[derive(
    Debug, PartialEq, Eq, Copy, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum SequencerType {
    /// The preferred sequencer with non-deferred execution privileges.
    Preferred,
    /// Any other sequencer, either registered with a standard registration or
    /// via emergency registration.
    NonPreferred,
}
