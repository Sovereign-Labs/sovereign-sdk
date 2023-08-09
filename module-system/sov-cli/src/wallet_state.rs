use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::clap;

/// A struct representing the current state of the CLI wallet
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned, Tx: Serialize + DeserializeOwned")]
pub struct WalletState<Tx, Ctx: sov_modules_api::Context> {
    /// The accumulated transactions to be submitted to the DA layer
    pub unsent_transactions: Vec<Tx>,
    /// The addresses in the wallet
    pub addresses: AddressList<Ctx>,
}

impl<Tx: Serialize + DeserializeOwned, Ctx: sov_modules_api::Context> WalletState<Tx, Ctx> {
    /// Load the wallet state from the given path on disk
    pub fn load(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref();
        if path.exists() {
            let data = fs::read(path)?;
            let state = serde_json::from_slice(data.as_slice())?;
            Ok(state)
        } else {
            Ok(Self {
                unsent_transactions: Vec::new(),
                addresses: AddressList {
                    other_addresses: Vec::new(),
                    active_address: None,
                },
            })
        }
    }

    /// Save the wallet state to the given path on disk
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }
}

/// A list of addresses associated with this wallet
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned")]
pub struct AddressList<Ctx: sov_modules_api::Context> {
    /// Any addresses which are known by the wallet but not currently active
    pub other_addresses: Vec<AddressEntry<Ctx>>,
    /// The address which is currently active
    pub active_address: Option<AddressEntry<Ctx>>,
}

impl<Ctx: sov_modules_api::Context> AddressList<Ctx> {
    /// Get the active address
    pub fn default_address(&self) -> Option<&AddressEntry<Ctx>> {
        self.active_address.as_ref()
    }
    /// Add an address to the wallet
    pub fn add(&mut self, address: Ctx::Address, nickname: Option<String>, location: PathBuf) {
        let entry = AddressEntry {
            address,
            nickname,
            location,
        };
        if self.active_address.is_none() {
            self.active_address = Some(entry);
        } else {
            self.other_addresses.push(entry);
        }
    }
}

/// An entry in the address list
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned")]
pub struct AddressEntry<Ctx: sov_modules_api::Context> {
    /// The address
    pub address: Ctx::Address,
    /// A user-provided nickname
    pub nickname: Option<String>,
    /// The location of the private key on disk
    pub location: PathBuf,
}

impl<Ctx: sov_modules_api::Context> AddressEntry<Ctx> {
    /// Check if the address entry matches the given nickname
    pub fn is_nicknamed(&self, nickname: &str) -> bool {
        self.nickname.as_deref() == Some(nickname)
    }

    /// Check if the address entry matches the given identifier
    pub fn matches(&self, identifier: &KeyIdentifier<Ctx>) -> bool {
        match identifier {
            KeyIdentifier::ByNickname { nickname } => self.is_nicknamed(nickname),
            KeyIdentifier::ByAddress { address } => &self.address == address,
        }
    }
}

/// An identifier for a key in the wallet
#[derive(Debug, clap::Subcommand, Clone)]
pub enum KeyIdentifier<C: sov_modules_api::Context> {
    /// Select a key by nickname
    ByNickname {
        /// The nickname
        nickname: String,
    },
    /// Select a key by its associated address
    ByAddress {
        /// The address
        address: C::Address,
    },
}
impl<C: sov_modules_api::Context> std::fmt::Display for KeyIdentifier<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyIdentifier::ByNickname { nickname } => nickname.fmt(f),
            KeyIdentifier::ByAddress { address } => address.fmt(f),
        }
    }
}
