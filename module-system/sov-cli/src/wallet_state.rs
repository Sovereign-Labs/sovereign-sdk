use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::clap;

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned, Tx: Serialize + DeserializeOwned")]
pub struct WalletState<Tx, Ctx: sov_modules_api::Context> {
    pub unsent_transactions: Vec<Tx>,
    pub addresses: AddressList<Ctx>,
}

impl<Tx: Serialize + DeserializeOwned, Ctx: sov_modules_api::Context> WalletState<Tx, Ctx> {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref();
        if path.exists() {
            let data = fs::read(path)?;
            let state = serde_json::from_slice(&mut data.as_slice())?;
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

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned")]
pub struct AddressList<Ctx: sov_modules_api::Context> {
    pub other_addresses: Vec<AddressEntry<Ctx>>,
    pub active_address: Option<AddressEntry<Ctx>>,
}

impl<Ctx: sov_modules_api::Context> AddressList<Ctx> {
    pub fn default_address(&self) -> Option<&AddressEntry<Ctx>> {
        self.active_address.as_ref()
    }
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "Ctx::Address: Serialize + DeserializeOwned")]
pub struct AddressEntry<Ctx: sov_modules_api::Context> {
    pub address: Ctx::Address,
    pub nickname: Option<String>,
    pub location: PathBuf,
}

impl<Ctx: sov_modules_api::Context> AddressEntry<Ctx> {
    pub fn is_nicknamed(&self, nickname: &str) -> bool {
        self.nickname.as_ref().map(|n| n.as_str()) == Some(nickname)
    }

    pub fn matches(&self, identifier: &KeyIdentifier<Ctx>) -> bool {
        match identifier {
            KeyIdentifier::ByNickname { nickname } => self.is_nicknamed(nickname),
            KeyIdentifier::ByAddress { address } => &self.address == address,
        }
    }
}

#[derive(Debug, clap::Subcommand, Clone)]
pub enum KeyIdentifier<C: sov_modules_api::Context> {
    ByNickname { nickname: String },
    ByAddress { address: C::Address },
}
impl<C: sov_modules_api::Context> std::fmt::Display for KeyIdentifier<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyIdentifier::ByNickname { nickname } => nickname.fmt(f),
            KeyIdentifier::ByAddress { address } => address.fmt(f),
        }
    }
}
