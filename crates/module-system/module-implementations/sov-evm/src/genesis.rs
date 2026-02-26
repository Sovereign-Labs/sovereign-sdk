use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_primitives::{Address, Bloom, B256, B64, U256};
use alloy_primitives::{BlockNumber, Bytes};
use anyhow::bail;
use revm::primitives::hardfork::SpecId;
use revm::state::AccountInfo;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::{GenesisState, Module, Spec};
use std::collections::BTreeMap;

use crate::conversions::create_block_env;
use crate::db::init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::{Evm, EvmGenesisConfig, EvmRuntimeConfig, EXCESS_BLOB_GAS};
#[cfg(feature = "native")]
use std::ops::RangeInclusive;

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

/// Evm account.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct AccountData {
    /// Account address.
    pub address: Address,
    /// Code hash.
    pub code_hash: B256,
    /// Smart contract code.
    pub code: Bytes,
    /// Account nonce.
    #[serde(default, skip_serializing_if = "is_default")]
    pub nonce: u64,
    /// Preloaded account storage values.
    #[serde(default, skip_serializing_if = "is_default")]
    pub storage: BTreeMap<U256, U256>,
}

impl AccountData {
    #[allow(missing_docs)]
    pub fn empty_code() -> B256 {
        KECCAK_EMPTY
    }

    #[allow(missing_docs)]
    pub fn balance(balance: u64) -> U256 {
        U256::from(balance)
    }

    /// Builds an empty EVM account for the given address.
    pub fn empty_with_address(address: Address) -> Self {
        AccountData {
            address,
            code_hash: KECCAK_EMPTY,
            code: Default::default(),
            nonce: 0,
            storage: Default::default(),
        }
    }
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.admin.set(&config.admin, state)?;
        let spec = init_spec(config)?;
        let chain_cfg = evm_chain_config(config, spec);

        let block = init_block(config);

        self.cfg.set(&chain_cfg, state)?;
        self.head.set(&block, state)?;

        let block_env = create_block_env(
            config.initial_base_fee,
            block.header.gas_limit,
            block.header.timestamp,
            block.header.beneficiary,
            block.header.number,
            None,
        );
        self.block_env.set(&block_env, state)?;
        for acc in config.accounts.clone() {
            self.init_account(acc, state)?;
        }

        #[cfg(feature = "native")]
        {
            self.block_numbers.set(&RangeInclusive::new(0, 0), state)?;
            self.pending_head.set(&block, state)?;
        }

        Ok(())
    }

    fn init_account(
        &mut self,
        acc: AccountData,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        let AccountData {
            address,
            code_hash,
            code,
            nonce,
            storage,
        } = acc;
        let mut evm_db = self.db(state);
        evm_db.insert_account_info(
            address,
            AccountInfo {
                balance: U256::ZERO,
                code_hash,
                nonce,
                code: None,
            },
        )?;

        if !code.is_empty() {
            evm_db.insert_code(code_hash, code)?;
        };

        for (slot, value) in storage {
            self.account_storage
                .set(&(&address, &slot), &value, state)?;
        }

        Ok(())
    }
}

fn init_block<S: Spec>(config: &EvmGenesisConfig<S>) -> Block {
    let header = alloy_consensus::Header {
        beneficiary: config.chain_spec.coinbase,
        // This will be set in finalize_hook or in the next begin_rollup_block_hook
        state_root: KECCAK_EMPTY,
        gas_limit: config.chain_spec.block_gas_limit,
        timestamp: config.genesis_timestamp,
        excess_blob_gas: Some(EXCESS_BLOB_GAS),
        base_fee_per_gas: Some(config.initial_base_fee),
        // Default values
        parent_hash: B256::ZERO,
        ommers_hash: EMPTY_OMMER_ROOT_HASH,
        transactions_root: EMPTY_ROOT_HASH,
        receipts_root: EMPTY_ROOT_HASH,
        logs_bloom: Bloom::default(),
        difficulty: U256::ZERO,
        number: 0,
        gas_used: 0,
        extra_data: Bytes::default(),
        mix_hash: B256::ZERO,
        nonce: B64::ZERO,
        withdrawals_root: None,
        blob_gas_used: None,
        parent_beacon_block_root: None,
        requests_hash: None,
    };

    Block {
        header,
        transactions: 0u64..0u64,
    }
}

fn init_spec<S: Spec>(config: &EvmGenesisConfig<S>) -> anyhow::Result<Vec<(BlockNumber, SpecId)>> {
    let mut spec = config.chain_spec.hardforks.to_vec();

    spec.sort_by(|a, b| a.0.cmp(&b.0));

    if spec.is_empty() {
        spec.push((0, SpecId::CANCUN));
    } else if spec[0].0 != 0u64 {
        bail!("EVM spec must start from block 0");
    };

    Ok(spec)
}

fn evm_chain_config<S: Spec>(
    cfg: &EvmGenesisConfig<S>,
    spec: Vec<(BlockNumber, SpecId)>,
) -> EvmRuntimeConfig {
    EvmRuntimeConfig {
        chain_spec: cfg.chain_spec.clone(),
        hardforks: spec,
        contract_creation_policy: cfg.contract_creation_policy.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_data_skips_default_nonce_and_storage_on_serialize() {
        let account = AccountData::empty_with_address(Address::from([1u8; 20]));
        let value = serde_json::to_value(account).unwrap();
        let obj = value.as_object().unwrap();
        assert!(
            !obj.contains_key("nonce"),
            "default nonce should be omitted from serialized genesis account"
        );
        assert!(
            !obj.contains_key("storage"),
            "empty storage should be omitted from serialized genesis account"
        );
    }

    #[test]
    fn account_data_serializes_non_default_nonce_and_storage() {
        let mut account = AccountData::empty_with_address(Address::from([2u8; 20]));
        account.nonce = 7;
        account.storage.insert(U256::from(1u64), U256::from(2u64));

        let value = serde_json::to_value(account).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.get("nonce"), Some(&serde_json::Value::from(7u64)));
        assert!(
            obj.get("storage")
                .and_then(serde_json::Value::as_object)
                .is_some(),
            "non-empty storage should be serialized"
        );
    }
}
