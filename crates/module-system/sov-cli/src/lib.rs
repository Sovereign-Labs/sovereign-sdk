#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
use std::env;
use std::path::Path;

use borsh::{BorshDeserialize, BorshSerialize};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
pub use sov_modules_api::clap;
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails, UnsignedTransaction};
use sov_modules_api::{Amount, DispatchCall, HexHash, HexString, Spec};
use sov_node_client as node_client;

/// Types and functionality storing and loading the persistent state of the wallet
pub mod wallet_state;
pub mod workflows;

pub use node_client::NodeClient;

const SOV_WALLET_DIR_ENV_VAR: &str = "SOV_WALLET_DIR";

/// The directory where the wallet is stored.
pub fn wallet_dir() -> anyhow::Result<impl AsRef<Path>> {
    // First try to parse from the env variable
    if let Ok(val) = env::var(SOV_WALLET_DIR_ENV_VAR) {
        return Ok(val.into());
    }

    // Fall back to the user's home directory
    let dir = BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory. You can set a wallet directory using the {} environment variable", SOV_WALLET_DIR_ENV_VAR))?
        .home_dir()
        .join(".sov_cli_wallet");

    Ok(dir)
}

/// An unsent transaction with the required data to be submitted to the DA layer
#[derive(Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, Clone)]
#[serde(bound = "Tx::Decodable: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransactionWithoutNonce<S: Spec, Tx>
where
    Tx: DispatchCall,
{
    // The underlying transaction
    tx: Tx::Decodable,
    // The chain root hash
    chain_hash: HexHash,
    // Details related to fees and gas handling.
    details: TxDetails<S>,
}

impl<S: Spec, Tx> UnsignedTransactionWithoutNonce<S, Tx>
where
    Tx: DispatchCall,
{
    /// Creates a new [`UnsignedTransactionWithoutNonce`] with the given arguments.
    pub const fn new(
        tx: Tx::Decodable,
        chain_id: u64,
        chain_hash: [u8; 32],
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        gas_limit: Option<S::Gas>,
    ) -> Self {
        Self {
            tx,
            chain_hash: HexString(chain_hash),
            details: TxDetails {
                max_priority_fee_bips,
                max_fee,
                gas_limit,
                chain_id,
            },
        }
    }

    /// Creates a new [`UnsignedTransaction`] from this [`UnsignedTransactionWithoutNonce`] when
    /// given a nonce.
    pub fn with_nonce(&self, nonce: u64) -> UnsignedTransaction<Tx, S> {
        UnsignedTransaction::new(
            self.tx.clone(),
            self.details.chain_id,
            self.details.max_priority_fee_bips,
            self.details.max_fee,
            nonce,
            self.details.gas_limit.clone(),
        )
    }
}
