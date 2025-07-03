use avail_rust::AccountId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvailData {
    pub data: Vec<u8>,
    pub signer: AccountId,
}
