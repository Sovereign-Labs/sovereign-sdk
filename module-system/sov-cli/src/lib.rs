use std::path::{Path, PathBuf};
use std::{env, fs};

use directories::BaseDirs;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
pub use sov_modules_api::clap;
use sov_modules_api::{PrivateKey, PublicKey, Spec};

const SOV_WALLET_DIR: &str = "SOV_WALLET_DIR";
const SOV_DEFAULT_KEY: &str = "SOV_DEFAULT_KEY";

/// The directory where the wallet is stored.
pub fn wallet_dir() -> Result<impl AsRef<Path>, anyhow::Error> {
    // First try to parse from the env variable
    if let Ok(val) = env::var(SOV_WALLET_DIR) {
        return Ok(PathBuf::try_from(val).map_err(|e| {
            anyhow::format_err!(
                "Error parsing directory from the '{SOV_WALLET_DIR}' environment variable: {}",
                e
            )
        })?);
    }

    // Fall back to the user's home directory
    let dir = BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory. You can set a wallet directory using the {} environment variable", SOV_WALLET_DIR))?
        .home_dir()
        .join(".sov_cli_wallet");

    Ok(dir)
}

/// The path to the private key file to use for signing transactions.
pub fn priv_key_path<Tx, Ctx: sov_modules_api::Context>(
    wallet_state: &WalletState<Tx, Ctx>,
) -> Result<impl AsRef<Path>, anyhow::Error> {
    if let Ok(val) = env::var(SOV_WALLET_DIR) {
        return Ok(PathBuf::try_from(val).map_err(|e| {
            anyhow::format_err!(
                "Error parsing location from the '{SOV_DEFAULT_KEY}' environment variable: {}",
                e
            )
        })?);
    }
    if let Some(addr) = wallet_state
        .addresses
        .default_address()
        .map(|entry| &entry.location)
    {
        return Ok(addr.clone());
    }
    anyhow::bail!("No default address found. You can set a default address using the {SOV_DEFAULT_KEY} environment variable, or import or generate a key using the `keys` subcommand", )
}

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

pub fn load_key<C: sov_modules_api::Context>(
    path: impl AsRef<Path>,
) -> Result<C::PrivateKey, anyhow::Error> {
    let data = fs::read(path)?;
    let key = serde_json::from_slice(&mut data.as_slice())?;
    Ok(key)
}

#[derive(clap::Subcommand)]
/// View and manage keys associated with this wallet
pub enum KeyWorkflow<C: sov_modules_api::Context> {
    /// Generate a new key pair
    Generate {
        #[clap(short, long)]
        /// A nickname for this key pair
        nickname: Option<String>,
    },
    /// Import an existing key pair
    Import {
        #[clap(short, long)]
        /// A nickname for this key pair
        nickname: Option<String>,
        #[clap(short, long)]
        /// Register a different address than the one that would be derived from the private key.
        address_override: Option<C::Address>,
        #[clap(short, long)]
        /// The path to the key file
        path: PathBuf,
    },
    /// List the keys in this wallet
    List,
    /// Set the active key
    Activate {
        #[clap(subcommand)]
        identifier: KeyIdentifier<C>,
    },
}

impl<C: sov_modules_api::Context> KeyWorkflow<C> {
    pub fn run<Tx: Serialize + DeserializeOwned>(
        self,
        wallet_state: &mut WalletState<Tx, C>,
        app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error> {
        match self {
            KeyWorkflow::Generate { nickname } => {
                let keys = <C as Spec>::PrivateKey::generate();
                let address = keys.pub_key().to_address::<<C as Spec>::Address>();
                let key_path = app_dir.as_ref().join(format!("{}.json", address));
                println!(
                    "Generated key pair with address: {}. Saving to {}",
                    address,
                    key_path.display()
                );
                std::fs::write(&key_path, serde_json::to_string(&keys)?)?;
                wallet_state.addresses.add(address, nickname, key_path);
            }
            KeyWorkflow::Import {
                nickname,
                address_override,
                path,
            } => {
                // Try to load the key as a sanity check.
                let key = load_key::<C>(&path)?;
                let address =
                    address_override.unwrap_or_else(|| key.pub_key().to_address::<C::Address>());
                println!("Imported key pair. address: {}", address);
                wallet_state.addresses.add(address, nickname, path);
            }
            KeyWorkflow::List => {
                println!("{}", serde_json::to_string_pretty(&wallet_state.addresses)?)
            }
            KeyWorkflow::Activate { identifier } => {
                if let Some(active) = wallet_state.addresses.active_address.as_mut() {
                    if active.matches(&identifier) {
                        println!("Key '{}' is already active", identifier);
                        return Ok(());
                    }
                    let requested = wallet_state
                        .addresses
                        .other_addresses
                        .iter_mut()
                        .find(|entry| entry.matches(&identifier))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Could not find key with nickname {}", identifier)
                        })?;
                    std::mem::swap(active, requested);
                    println!("Activated key {}", identifier);
                }
            }
        }
        Ok(())
    }
}
