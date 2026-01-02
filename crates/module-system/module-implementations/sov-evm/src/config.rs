use alloy_primitives::Address;
use borsh::{BorshDeserialize, BorshSerialize};
use revm::primitives::hardfork::SpecId;
use schemars::JsonSchema;
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexString, SafeVec, Spec, ETHEREUM_BLOCK_GAS_LIMIT, ETHEREUM_TX_GAS_LIMIT};
use sov_universal_wallet::UniversalWallet;
use std::collections::BTreeSet;

use crate::AccountData;

/// Core EVM chain parameters shared between genesis and runtime
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmChainSpec {
    /// Maximum contract code size (None = default. Currently 512KiB)
    pub limit_contract_code_size: Option<usize>,
    /// Address where transaction fees are collected
    pub coinbase: Address,
    /// Maximum gas allowed per block
    pub block_gas_limit: u64,
    /// Maximum gas allowed per tx. Defaults to block gas limit if none is provided.
    #[serde(default)]
    pub tx_gas_limit: Option<u64>,
    /// Hard fork activation schedule (block number -> fork ID)
    pub hardforks: Vec<(u64, SpecId)>,
}

/// Genesis configuration for EVM module initialization
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmGenesisConfig<S: Spec> {
    /// Initial account states
    pub accounts: Vec<AccountData>,
    /// Initial base fee for first block
    pub initial_base_fee: u64,
    /// Timestamp of genesis block
    pub genesis_timestamp: u64,
    /// Core chain parameters
    pub chain_spec: EvmChainSpec,
    /// Policy - who can create contracts. Everyone or allowlist
    pub contract_creation_policy: ContractCreationPolicy,
    /// The address which is allowed to modify the config.
    pub admin: S::Address,
}

impl Default for EvmChainSpec {
    fn default() -> Self {
        Self {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT,
            tx_gas_limit: Some(ETHEREUM_TX_GAS_LIMIT),
            hardforks: vec![(0, SpecId::CANCUN)],
        }
    }
}

/// Policy - who can create contracts. Everyone or allowlist
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContractCreationPolicy {
    /// No restrictions on contract creation
    #[default]
    Everyone,
    /// Only allowed addresses can create contracts
    Allowlist(BTreeSet<Address>),
}

impl ContractCreationPolicy {
    /// Returns true if address is allowed to deploy contracts by the policy. False otherwise
    pub fn allows(&self, address: &Address) -> bool {
        match self {
            Self::Everyone => true,
            Self::Allowlist(allowlist) => allowlist.contains(address),
        }
    }

    /// Returns the allowlist, leaving an empty list in its place. If the policy is Everyone, returns an empty list.
    pub fn take_allowlist(&mut self) -> BTreeSet<Address> {
        match self {
            Self::Allowlist(allowlist) => std::mem::take(allowlist),
            Self::Everyone => BTreeSet::new(),
        }
    }
}

/// Runtime configuration for EVM execution
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EvmRuntimeConfig {
    /// Core chain parameters
    pub chain_spec: EvmChainSpec,
    /// Sorted hard fork schedule for efficient runtime lookup
    /// (block number, fork ID) ordered by block number
    pub hardforks: Vec<(u64, SpecId)>,
    /// Policy - who can create contracts. Everyone or allowlist
    pub contract_creation_policy: ContractCreationPolicy,
}

impl Default for EvmRuntimeConfig {
    fn default() -> EvmRuntimeConfig {
        let chain_spec = EvmChainSpec::default();
        // Clone hardforks from chain_spec for runtime use
        let hardforks = chain_spec.hardforks.clone();

        EvmRuntimeConfig {
            chain_spec,
            hardforks,
            contract_creation_policy: ContractCreationPolicy::Everyone,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(transparent)]
/// A wrapper around `SpecId` that implements Borsh serialization and deserialization.
pub struct BorshSpecId(#[schemars(with = "String")] pub SpecId);

impl BorshSerialize for BorshSpecId {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        let id = self.0 as u8;
        id.serialize(writer)
    }
}

impl BorshDeserialize for BorshSpecId {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let id = u8::deserialize_reader(reader)?;
        Ok(Self(SpecId::try_from(id).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?))
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
    JsonSchema,
)]
#[serde(bound = "S: Spec")]
#[schemars(bound = "S: Spec", rename = "evm_runtime_config_update")]
/// An update to the runtime configuration.
pub struct EvmRuntimeConfigUpdate<S: Spec> {
    // Validation: ensure SpecId > current_spec_id and activation block number is greater than the current block number
    /// A new hardfork to activate and the block number at which it activates
    #[sov_wallet(as_ty = "Option<(u64, u8)>")]
    pub new_hardfork: Option<(u64, BorshSpecId)>,
    /// A new contract creation policy to apply. None means "no change"
    pub new_contract_creation_policy: Option<ContractCreationPolicyUpdate>,
    /// A new chain spec to apply. None means "no change"
    pub chain_spec_update: Option<ChainSpecUpdate>,
    /// A new admin address to set. None means "no change"
    pub new_admin: Option<S::Address>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    UniversalWallet,
    JsonSchema,
)]
/// An update to the chain spec.
pub struct ChainSpecUpdate {
    /// The new limit for contract code size. None means "no change"
    // Check that the limit is less than 10MB
    pub new_limit_contract_code_size: Option<usize>,
    /// The new block gas limit. Must be greater than 5M to avoid censorship. None means "no change"
    /// Check that the limit is greater than 5M to avoid accidental complete shutdown.
    pub new_block_gas_limit: Option<u64>,
    /// The new tx gas limit. Must be less than or equal to the effective block gas limit after applying the update. None means "no change"
    pub new_tx_gas_limit: Option<u64>,
}

/// Policy - who can create contracts. Everyone or allowlist
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    Default,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
    JsonSchema,
)]
#[serde(rename = "contract_creation_policy_update")] // rename_all isn't supported in UniversalWallet yet
pub enum ContractCreationPolicyUpdate {
    /// No restrictions on contract creation
    #[default]
    #[serde(rename = "everyone")]
    Everyone,
    /// Only allowed addresses can create contracts
    #[serde(rename = "allowlist")]
    Allowlist {
        /// Addresses to add to the allowlist
        add: SafeVec<HexString<[u8; 20]>, 32>,
        /// Addresses to remove from the allowlist
        remove: SafeVec<HexString<[u8; 20]>, 32>,
    },
}

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub(crate) enum GasMeteringMode {
    /// EVM doesn't charge for storage access and initial cost.
    /// Sequencer charges the initial cost and state access.
    /// Later adds to the receipt amount.
    #[default]
    Rollup,
    /// EVM gas costs are resembling those on the mainnet.
    /// Useful to compute metrics like MGas/s.
    Evm,
}

impl From<&str> for GasMeteringMode {
    fn from(s: &str) -> Self {
        match s {
            "Rollup" => GasMeteringMode::Rollup,
            "EVM" => GasMeteringMode::Evm,
            _ => panic!("Invalid EVM_GAS_METERING_MODE"),
        }
    }
}

pub(crate) fn gas_metering_mode() -> GasMeteringMode {
    config_value!("EVM_GAS_METERING_MODE").into()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{Address, Bytes};
    use revm::primitives::hardfork::SpecId;
    use sov_modules_api::prelude::serde_json;
    use sov_test_utils::TestSpec;

    use crate::{AccountData, EvmChainSpec, EvmGenesisConfig};

    #[test]
    fn test_config_serialization() {
        let address = Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();
        let config = EvmGenesisConfig::<TestSpec> {
            accounts: vec![AccountData {
                address,
                code_hash: AccountData::empty_code(),
                code: Bytes::default(),
            }],
            chain_spec: EvmChainSpec {
                limit_contract_code_size: None,
                hardforks: vec![(0, SpecId::CANCUN)],
                ..Default::default()
            },
            genesis_timestamp: 0,
            contract_creation_policy: Default::default(),
            initial_base_fee: 7,
            admin: sov_modules_api::Address::from_str(
                "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
            )
            .unwrap(),
        };

        let data = r#"
        {
            "accounts":[
                {
                    "address":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                    "code_hash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
                    "code":"0x",
                    "nonce":0
                }],
                "initial_base_fee":7,
                "genesis_timestamp":0,
                "chain_spec":{
                    "limit_contract_code_size":null,
                    "coinbase":"0x0000000000000000000000000000000000000000",
                    "block_gas_limit":1000000000,
                    "tx_gas_limit":30000000,
                    "hardforks":[[0,"CANCUN"]]
                },
                "contract_creation_policy": "everyone",
                "admin": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
        }"#;

        let parsed_config: EvmGenesisConfig<TestSpec> = serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config);
    }
}
