use std::path::{Path, PathBuf};
use std::{fs, mem};

use anyhow::Context;
use semver::Version;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{clap, CredentialId, CryptoSpec, DispatchCall, PrivateKey, PublicKey};

use crate::UnsignedTransactionWithoutNonce;

const NO_ACCOUNTS_FOUND: &str =
    "No accounts found. You can generate one with the `keys generate` subcommand";

/// A struct representing the current state of the CLI wallet
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    bound = "S::Address: Serialize + DeserializeOwned, Tx::Decodable: Serialize + DeserializeOwned"
)]
pub struct WalletState<Tx, S: sov_modules_api::Spec>
where
    Tx: DispatchCall,
{
    /// The accumulated transactions to be submitted to the DA layer.
    pub unsent_transactions: Vec<UnsignedTransactionWithoutNonce<S, Tx>>,
    /// The addresses in the wallet
    pub addresses: AddressList<S>,
    /// The REST API URL
    pub rest_api_url: Option<String>,
    /// The version of the library that serialized the state.
    pub version: String,
}

impl<Tx, S> Default for WalletState<Tx, S>
where
    Tx: DispatchCall,
    S: sov_modules_api::Spec,
{
    fn default() -> Self {
        Self {
            unsent_transactions: Vec::new(),
            addresses: AddressList {
                addresses: Vec::new(),
            },
            rest_api_url: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl<Tx, S> WalletState<Tx, S>
where
    Tx: DispatchCall,
    Tx::Decodable: Serialize + DeserializeOwned,
    S: sov_modules_api::Spec,
{
    /// Load the wallet state from the given path on disk
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            let data = fs::read(path)?;

            let version = env!("CARGO_PKG_VERSION")
                .parse::<Version>()
                .expect("Failed to parse the library version");

            let value: Value = serde_json::from_slice(data.as_slice()).map_err(|e|
                anyhow::anyhow!(
                    "Failed to read the JSON state of the wallet. Check if `{}` points to a valid JSON state file. Error: {e}",
                    path.display()
                )
            )?;

            let data_version = value
                .get("version")
                .and_then(Value::as_str)
                .and_then(|v| v.parse::<Version>().ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Failed to read the version from state of the wallet. Check if `{}` points to a valid JSON state file.",
                        path.display()
                    )
                })?;

            if version.major != data_version.major
                || version.major == 0 && version.minor != data_version.minor
            {
                anyhow::bail!(
                    "The version that created the state on the state file `{}` is `{data_version}`, and the library is `{version}`.

This discrepancy may result in data layout inconsistency. Consider one of the following options:

- Migrate the wallet state to the current version. Check the repository documentation.
- Use a different data directory via the environment variable `SOV_WALLET_DIR_ENV_VAR` to generate an empty state with the current version.
- Delete the state file` so the wallet will generate a new empty state.
- Manually update the version on the state file to `{version}`. Warning: this approach assumes the data layout to be the same.",
                    path.display(),
                );
            }

            let state = serde_json::from_slice(data.as_slice())?;

            Ok(state)
        } else {
            Ok(Default::default())
        }
    }

    /// Save the wallet state to the given path on disk
    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    /// Returns the serialized, signed transactions of the state.
    ///
    /// Consumes unsigned transactions, signing them with the provided key and using the supplied
    /// nonce for each transaction, incrementally.
    pub fn take_signed_transactions(
        &mut self,
        signing_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
        nonce: u64,
    ) -> Vec<Vec<u8>> {
        mem::take(&mut self.unsent_transactions)
            .into_iter()
            .enumerate()
            .map(|(offset, tx)| {
                let nonce = nonce.checked_add(offset as u64).expect("Nonce overflow");
                sign_tx(signing_key, &tx, nonce).expect("Tx signing failed")
            })
            .collect()
    }

    /// Resolving id into AddressEntry
    pub fn resolve_account(
        &mut self,
        account_id: Option<&KeyIdentifier<S>>,
    ) -> anyhow::Result<&AddressEntry<S>> {
        match account_id {
            None => self
                .addresses
                .default_address()
                .ok_or_else(|| anyhow::format_err!(NO_ACCOUNTS_FOUND)),
            Some(id) => {
                let addr = self.addresses.get_address(id);

                addr.ok_or_else(|| {
                    anyhow::format_err!("No account found matching identifier: {}", id)
                })
            }
        }
    }
}

/// Returns borsh serialized [`Transaction`].
pub(crate) fn sign_tx<S, Tx>(
    signing_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
    tx: &UnsignedTransactionWithoutNonce<S, Tx>,
    nonce: u64,
) -> anyhow::Result<Vec<u8>>
where
    S: sov_modules_api::Spec,
    Tx: DispatchCall,
{
    let tx = Transaction::<Tx, S>::new_signed_tx(
        signing_key,
        &tx.chain_hash.into(),
        tx.with_nonce(nonce),
    );
    let tx = borsh::to_vec(&tx)?;
    Ok(tx)
}

/// A struct representing private key and associated address
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
pub struct PrivateKeyAndAddress<S: sov_modules_api::Spec> {
    /// Private key of the address
    pub private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
    /// Address associated from the private key
    pub address: S::Address,
}

impl<S: sov_modules_api::Spec> PrivateKeyAndAddress<S> {
    /// Returns boolean if the private key matches default address
    pub fn is_matching_to_default(&self) -> bool {
        let pub_key = &self.private_key.pub_key();
        let credential_id: CredentialId = pub_key.credential_id();
        let addr: S::Address = credential_id.into();
        addr == self.address
    }

    /// Randomly generates a new private key and address
    pub fn generate() -> Self {
        let private_key = <S::CryptoSpec as CryptoSpec>::PrivateKey::generate();
        let credential_id: CredentialId = private_key.pub_key().credential_id();
        let address: S::Address = credential_id.into();
        Self {
            private_key,
            address,
        }
    }

    /// Generates a valid private key and address from a given private key
    pub fn from_key(private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey) -> Self {
        let credential_id: CredentialId = private_key.pub_key().credential_id();
        let address: S::Address = credential_id.into();
        Self {
            private_key,
            address,
        }
    }

    /// Deserializes from json file.
    pub fn from_json_file(path: impl AsRef<Path>, check_is_default: bool) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path.as_ref())?;

        let key_and_address: PrivateKeyAndAddress<S> = serde_json::from_str(&data)
            .with_context(|| format!("Unable to convert data {} to PrivateKeyAndAddress", &data))?;

        if check_is_default {
            anyhow::ensure!(
                key_and_address.is_matching_to_default(),
                "Key default address does not match address in file. Probably an error in file"
            );
        }

        Ok(key_and_address)
    }
}

/// A list of addresses associated with this wallet
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
pub struct AddressList<S: sov_modules_api::Spec> {
    /// All addresses which are known by the wallet. The active address is stored
    /// first in this array
    addresses: Vec<AddressEntry<S>>,
}

impl<S: sov_modules_api::Spec> AddressList<S> {
    /// Get the active address
    pub fn default_address(&self) -> Option<&AddressEntry<S>> {
        self.addresses.first()
    }

    /// Get an address by identifier
    pub fn get_address(&mut self, identifier: &KeyIdentifier<S>) -> Option<&AddressEntry<S>> {
        self.addresses
            .iter()
            .find(|entry| entry.matches(identifier))
    }

    /// Activate a key by identifier
    pub fn activate(&mut self, identifier: &KeyIdentifier<S>) -> Option<&AddressEntry<S>> {
        let (idx, _) = self
            .addresses
            .iter()
            .enumerate()
            .find(|(_idx, entry)| entry.matches(identifier))?;
        self.addresses.swap(0, idx);
        self.default_address()
    }

    /// Remove an address from the wallet by identifier
    pub fn remove(&mut self, identifier: &KeyIdentifier<S>) {
        self.addresses.retain(|entry| !entry.matches(identifier));
    }

    /// Add an address to the wallet
    pub fn add(
        &mut self,
        address: S::Address,
        nickname: Option<String>,
        public_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        location: PathBuf,
    ) -> anyhow::Result<()> {
        if nickname.is_some()
            && self
                .addresses
                .iter()
                .any(|entry| entry.nickname == nickname)
        {
            anyhow::bail!("Key with nickname '{}' already exists", nickname.unwrap());
        }
        let entry = AddressEntry {
            address,
            nickname,
            location,
            pub_key: public_key,
        };
        self.addresses.push(entry);

        Ok(())
    }

    /// Returns the number of addresses in the list.
    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    /// Returns if [`AddressList`] is empty or not.
    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }
}

/// An entry in the address list
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
pub struct AddressEntry<S: sov_modules_api::Spec> {
    /// The address
    pub address: S::Address,
    /// A user-provided nickname
    pub nickname: Option<String>,
    /// The location of the private key on disk
    pub location: PathBuf,
    /// The public key associated with the address
    #[serde(with = "pubkey_serde")]
    pub pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
}

impl<S: sov_modules_api::Spec> AddressEntry<S> {
    /// Check if the address entry matches the given nickname
    pub fn is_nicknamed(&self, nickname: &str) -> bool {
        self.nickname.as_deref() == Some(nickname)
    }

    /// Check if the address entry matches the given identifier
    pub fn matches(&self, identifier: &KeyIdentifier<S>) -> bool {
        match identifier {
            KeyIdentifier::ByNickname { nickname } => self.is_nicknamed(nickname),
            KeyIdentifier::ByAddress { address } => &self.address == address,
        }
    }
}

/// An identifier for a key in the wallet
#[derive(Debug, clap::Subcommand, Clone, derive_more::Display)]
pub enum KeyIdentifier<S: sov_modules_api::Spec> {
    /// Select a key by nickname
    ByNickname {
        /// The nickname
        nickname: String,
    },
    /// Select a key by its associated address
    ByAddress {
        /// The address
        address: S::Address,
    },
}

mod pubkey_serde {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use sov_modules_api::PublicKey;
    use sov_rollup_interface::common::HexString;

    pub fn serialize<P: PublicKey + borsh::BorshSerialize, S>(
        data: &P,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes = borsh::to_vec(data).expect("serialization to vec is infallible");
        HexString::new(bytes).serialize(serializer)
    }

    /// Deserializes a hex string into raw bytes.
    ///
    /// Both upper and lower case characters are valid in the input string and can
    /// even be mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all valid strings).
    pub fn deserialize<'de, C, D>(deserializer: D) -> Result<C, D::Error>
    where
        C: PublicKey + borsh::BorshDeserialize,
        D: Deserializer<'de>,
    {
        let hex_s = HexString::<Vec<u8>>::deserialize(deserializer)?;
        C::try_from_slice(&hex_s.0).map_err(Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::Spec;

    use super::*;

    type S = sov_test_utils::TestSpec;

    #[test]
    fn to_string() {
        assert_eq!(
            KeyIdentifier::<S>::ByNickname {
                nickname: "test".to_string()
            }
            .to_string(),
            "test"
        );

        assert_eq!(
            KeyIdentifier::<S>::ByAddress {
                address: <S as Spec>::Address::from([0; 28])
            }
            .to_string(),
            "sov1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq7xm4r8"
        );
    }

    #[test]
    fn test_private_key_and_address() {
        let private_key_and_address = PrivateKeyAndAddress::<S>::generate();

        let json = serde_json::to_string_pretty(&private_key_and_address).unwrap();

        let decoded: PrivateKeyAndAddress<S> = serde_json::from_str(&json).unwrap();

        assert_eq!(
            private_key_and_address.private_key.pub_key(),
            decoded.private_key.pub_key()
        );
        assert_eq!(private_key_and_address.address, decoded.address);
    }
}
