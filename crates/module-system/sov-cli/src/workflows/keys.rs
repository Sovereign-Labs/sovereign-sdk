//! Key management workflows for the sov CLI wallet
use std::path::{Path, PathBuf};

use sov_modules_api::{clap, CredentialId, CryptoSpec, DispatchCall, PrivateKey, PublicKey};

use crate::wallet_state::{KeyIdentifier, PrivateKeyAndAddress, WalletState};

#[derive(clap::Subcommand)]
/// View and manage keys associated with this wallet.
pub enum KeyWorkflow<S: sov_modules_api::Spec> {
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
        address_override: Option<S::Address>,
        #[clap(short, long)]
        /// The path to the key file
        path: PathBuf,
        #[arg(short, long)]
        /// Import is not performed is key with given nickname or address already in the store.
        skip_if_present: bool,
    },
    /// List the keys in this wallet
    List,
    /// Set the active key
    Activate {
        /// The identifier of the key to activate
        #[clap(subcommand)]
        identifier: KeyIdentifier<S>,
    },
    /// Unlink a key from the wallet
    Remove {
        /// The identifier of the key to remove
        #[clap(subcommand)]
        identifier: KeyIdentifier<S>,
    },
    /// Show a key info from the wallet
    Show {
        /// The identifier of the key to show
        #[clap(subcommand)]
        identifier: KeyIdentifier<S>,
    },
}

impl<S: sov_modules_api::Spec> KeyWorkflow<S> {
    /// Run the key workflow to import, generate, activate, remove or list keys.
    /// WalletState shouldn't be saved in case of Error.
    pub fn run<Tx>(
        self,
        wallet_state: &mut WalletState<Tx, S>,
        app_dir: impl AsRef<Path>,
    ) -> anyhow::Result<()>
    where
        Tx: DispatchCall,
    {
        match self {
            KeyWorkflow::Generate { nickname } => {
                generate_and_save_key(nickname, app_dir, wallet_state)?;
            }
            KeyWorkflow::Import {
                nickname,
                address_override,
                path,
                skip_if_present,
            } => {
                // Try to load the key as a sanity check.
                let private_key = load_key::<S>(&path)?;
                let public_key = private_key.pub_key();

                let credential_id: CredentialId = public_key.credential_id();
                let default_address: S::Address = credential_id.into();

                let address = address_override.unwrap_or(default_address);

                if skip_if_present {
                    let identifier = if let Some(nickname) = nickname.clone() {
                        KeyIdentifier::ByNickname { nickname }
                    } else {
                        KeyIdentifier::ByAddress {
                            address: address.clone(),
                        }
                    };
                    if wallet_state.addresses.get_address(&identifier).is_some() {
                        tracing::info!(
                            "A key with identifier {} already exists. Skipping import!",
                            identifier
                        );
                        return Ok(());
                    }
                }
                wallet_state
                    .addresses
                    .add(address.clone(), nickname, public_key, path)?;
                tracing::info!("Imported key pair. address: {}", address);
            }
            KeyWorkflow::Show { identifier } => {
                let addr = wallet_state.addresses.get_address(&identifier);
                // keep this as println, because it is consumed by external command line tools.
                println!("{}", serde_json::to_string_pretty(&addr)?);
            }
            KeyWorkflow::List => {
                // keep this as println, because it is consumed by external command line tools.
                println!("{}", serde_json::to_string_pretty(&wallet_state.addresses)?);
            }
            KeyWorkflow::Activate { identifier } => {
                if let Some(active) = wallet_state.addresses.default_address() {
                    if active.matches(&identifier) {
                        tracing::warn!(%identifier, "Key is already active");
                        return Ok(());
                    }
                }
                wallet_state
                    .addresses
                    .activate(&identifier)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Could not find key with identifier {}", identifier)
                    })?;
                tracing::info!(%identifier, "Activated key");
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
pub fn load_key<S: sov_modules_api::Spec>(
    path: impl AsRef<Path>,
) -> anyhow::Result<<S::CryptoSpec as CryptoSpec>::PrivateKey> {
    let data = std::fs::read_to_string(path)?;
    let key_and_address: PrivateKeyAndAddress<S> = serde_json::from_str(&data)?;
    Ok(key_and_address.private_key)
}

/// Generate a new key pair and save it to the wallet
pub fn generate_and_save_key<Tx, S: sov_modules_api::Spec>(
    nickname: Option<String>,
    app_dir: impl AsRef<Path>,
    wallet_state: &mut WalletState<Tx, S>,
) -> anyhow::Result<()>
where
    Tx: DispatchCall,
{
    let keys = <S::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    let key_and_address = PrivateKeyAndAddress::<S>::from_key(keys);
    let public_key = key_and_address.private_key.pub_key();
    let address = key_and_address.address.clone();
    let key_path = app_dir.as_ref().join(format!("{address}.json"));
    // First try to serialize, before making anything dirty
    let serialized_key = serde_json::to_string(&key_and_address)?;
    // Trying to add key state
    wallet_state
        .addresses
        .add(address.clone(), nickname, public_key, key_path.clone())?;
    tracing::info!(
        %address,
        path = %key_path.display(),
        "Generated key pair with address and saving to path",
    );
    // If this fails, caller should not save the errored wallet state
    std::fs::write(&key_path, serialized_key)?;
    Ok(())
}
