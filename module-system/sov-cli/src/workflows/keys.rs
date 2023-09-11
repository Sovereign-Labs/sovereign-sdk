//! Key management workflows for the sov CLI wallet
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::{clap, PrivateKey, PublicKey, Spec};

use crate::wallet_state::{KeyIdentifier, PrivateKeyAndAddress, WalletState};

#[derive(clap::Subcommand)]
/// View and manage keys associated with this wallet
pub enum KeyWorkflow<C: sov_modules_api::Context> {
    /// Generate a new key pair
    Generate {
        #[clap(short, long)]
        /// A nickname for this key pair
        nickname: Option<String>,
    },
    /// Generate a new key pair if none exist
    GenerateIfMissing {
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
    /// Unlink a key from the wallet
    Remove {
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
                generate_and_save_key(nickname, app_dir, wallet_state)?;
            }
            KeyWorkflow::Import {
                nickname,
                address_override,
                path,
            } => {
                // Try to load the key as a sanity check.
                let private_key = load_key::<C>(&path)?;
                let public_key = private_key.pub_key();
                let address =
                    address_override.unwrap_or_else(|| public_key.to_address::<C::Address>());
                println!("Imported key pair. address: {}", address);
                wallet_state
                    .addresses
                    .add(address, nickname, public_key, path);
            }
            KeyWorkflow::List => {
                println!("{}", serde_json::to_string_pretty(&wallet_state.addresses)?)
            }
            KeyWorkflow::Activate { identifier } => {
                if let Some(active) = wallet_state.addresses.default_address() {
                    if active.matches(&identifier) {
                        println!("Key '{}' is already active", identifier);
                        return Ok(());
                    }
                }
                wallet_state
                    .addresses
                    .activate(&identifier)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Could not find key with identifier {}", identifier)
                    })?;
                println!("Activated key {}", identifier);
            }
            KeyWorkflow::GenerateIfMissing { nickname } => {
                if wallet_state.addresses.default_address().is_none() {
                    generate_and_save_key(nickname, app_dir, wallet_state)?;
                }
            }
            KeyWorkflow::Remove { identifier } => {
                wallet_state.addresses.remove(&identifier);
            }
        }
        Ok(())
    }
}

/// Load a key from the given path
pub fn load_key<C: sov_modules_api::Context>(
    path: impl AsRef<Path>,
) -> Result<C::PrivateKey, anyhow::Error> {
    let data = std::fs::read_to_string(path)?;
    let key_and_address: PrivateKeyAndAddress<C> = serde_json::from_str(&data)?;
    Ok(key_and_address.private_key)
}

/// Generate a new key pair and save it to the wallet
pub fn generate_and_save_key<Tx, C: sov_modules_api::Context>(
    nickname: Option<String>,
    app_dir: impl AsRef<Path>,
    wallet_state: &mut WalletState<Tx, C>,
) -> Result<(), anyhow::Error> {
    let keys = <C as Spec>::PrivateKey::generate();
    let key_and_address = PrivateKeyAndAddress::<C>::from_key(keys);
    let public_key = key_and_address.private_key.pub_key();
    let address = key_and_address.address.clone();
    let key_path = app_dir.as_ref().join(format!("{}.json", address));
    println!(
        "Generated key pair with address: {}. Saving to {}",
        address,
        key_path.display()
    );
    std::fs::write(&key_path, serde_json::to_string(&key_and_address)?)?;
    wallet_state
        .addresses
        .add(address, nickname, public_key, key_path);
    Ok(())
}
