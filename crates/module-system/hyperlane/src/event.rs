use sov_modules_api::{HexHash, HexString};

use crate::types::Domain;
use crate::{EthAddress, StorageLocation};

/// Sample Event
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Clone,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// A complete summary of a message, emitted when a message has been dispatched (aka "sent").
    /// Used by the relayer to extract all info needed for relaying.
    Dispatch {
        /// The sender of the message.
        sender: HexHash,
        /// The destination domain of the message.
        destination_domain: Domain,
        /// The recipient address of the message.
        recipient_address: HexHash,
        /// The message body.
        message: HexString,
    },
    /// A smaller event containing only the message ID, emitted when a message has been dispatched (aka "sent").
    /// Used by the relayer when the full message data is unnecessary.
    DispatchId {
        /// The ID of the message.
        id: HexHash,
    },
    /// A message has been received and processed.
    Process {
        /// The origin domain of the message.
        origin_domain: Domain,
        /// The sender address of the message.
        sender_address: HexHash,
        /// The recipient address of the message.
        recipient_address: HexHash,
    },
    /// A smaller event containing only the message ID, emitted when a message has been received and processed.
    /// Used by the relayer when the full message data is unnecessary.
    ProcessId {
        /// The ID of the message.
        id: HexHash,
    },
    /// Announcement of validator and its signatures location.
    ValidatorAnnouncement {
        /// Ethereum address of the validator.
        address: EthAddress,
        /// Storage for the validator's signatures.
        storage_location: StorageLocation,
    },
}
