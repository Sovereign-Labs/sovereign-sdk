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
    /// The addresses in the wallet
    pub rpc_url: Option<String>,
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
                rpc_url: None,
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

    /// Get an address by identifier
    pub fn get_address(
        &mut self,
        identifier: &KeyIdentifier<Ctx>,
    ) -> Option<&mut AddressEntry<Ctx>> {
        if let Some(active) = self.active_address.as_mut() {
            if active.matches(&identifier) {
                return Some(active);
            }
            self.other_addresses
                .iter_mut()
                .find(|entry| entry.matches(&identifier))
        } else {
            None
        }
    }
    /// Add an address to the wallet
    pub fn add(
        &mut self,
        address: Ctx::Address,
        nickname: Option<String>,
        public_key: Ctx::PublicKey,
        location: PathBuf,
    ) {
        let entry = AddressEntry {
            address,
            nickname,
            location,
            pub_key: public_key,
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
    /// The public key associated with the address
    #[serde(with = "pubkey_hex")]
    pub pub_key: Ctx::PublicKey,
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

mod pubkey_hex {
    use core::fmt;
    use std::marker::PhantomData;

    use borsh::{BorshDeserialize, BorshSerialize};
    use hex::{FromHex, ToHex};
    use serde::de::{Error, Visitor};
    use serde::{Deserializer, Serializer};
    use sov_modules_api::PublicKey;
    pub fn serialize<P: PublicKey + BorshSerialize, S>(
        data: &P,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes = data
            .try_to_vec()
            .expect("serialization to vec is infallible");
        let formatted_string = format!("0x{}", bytes.encode_hex::<String>());
        serializer.serialize_str(&formatted_string)
    }

    /// Deserializes a hex string into raw bytes.
    ///
    /// Both, upper and lower case characters are valid in the input string and can
    /// even be mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all valid strings).
    pub fn deserialize<'de, D, C>(deserializer: D) -> Result<C, D::Error>
    where
        D: Deserializer<'de>,
        C: PublicKey + BorshDeserialize,
    {
        struct HexPubkeyVisitor<C>(PhantomData<C>);

        impl<'de, C: PublicKey + BorshDeserialize> Visitor<'de> for HexPubkeyVisitor<C> {
            type Value = C;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a hex encoded string")
            }

            fn visit_str<E>(self, data: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                let bytes: Vec<u8> = FromHex::from_hex(data).map_err(Error::custom)?;
                C::try_from_slice(&bytes).map_err(Error::custom)
            }

            fn visit_borrowed_str<E>(self, data: &'de str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                let bytes: Vec<u8> = FromHex::from_hex(data).map_err(Error::custom)?;
                C::try_from_slice(&bytes).map_err(Error::custom)
            }
        }

        deserializer.deserialize_str(HexPubkeyVisitor(PhantomData::<C>))
    }
}
