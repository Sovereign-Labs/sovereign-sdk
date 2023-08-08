//! Key management workflows for the sov CLI wallet
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::{clap, PrivateKey, PublicKey, Spec};

use crate::wallet_state::{KeyIdentifier, WalletState};

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
        /// The identifier of the key to activate
        #[clap(subcommand)]
        identifier: KeyIdentifier<C>,
    },
}

impl<C: sov_modules_api::Context> KeyWorkflow<C> {
    /// Run the key workflow to import, generate, activate, or list keys
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

/// Load a key from the given path
pub fn load_key<C: sov_modules_api::Context>(
    path: impl AsRef<Path>,
) -> Result<C::PrivateKey, anyhow::Error> {
    let data = std::fs::read(path)?;
    let key = serde_json::from_slice(data.as_slice())?;
    Ok(key)
}
