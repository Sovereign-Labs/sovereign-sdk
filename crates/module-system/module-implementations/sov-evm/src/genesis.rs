use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{Address, B256, U256};
use alloy_primitives::{BlockNumber, Bytes};
use anyhow::Result;
use revm::primitives::hardfork::SpecId;
use revm::state::AccountInfo;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::{GenesisState, Module, Spec};

use crate::db::init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::{Evm, EvmGenesisConfig, EvmRuntimeConfig};
#[cfg(feature = "native")]
use std::ops::RangeInclusive;

/// Evm account.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct AccountData {
    /// Account address.
    pub address: Address,
    /// Code hash.
    pub code_hash: B256,
    /// Smart contract code.
    pub code: Bytes,
}

impl AccountData {
    /// Empty code hash.
    pub fn empty_code() -> B256 {
        KECCAK_EMPTY
    }

    /// Account balance.
    pub fn balance(balance: u64) -> U256 {
        U256::from(balance)
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
    ) -> Result<()> {
        for acc in config.accounts.clone() {
            self.init_account(acc, state)?;
        }

        let spec = init_spec(config)?;
        let chain_cfg = evm_chain_config(config, spec);
        let block = init_block(config);

        self.cfg.set(&chain_cfg, state)?;
        self.head.set(&block, state)?;
        #[cfg(feature = "native")]
        {
            self.block_numbers.set(&RangeInclusive::new(0, 0), state)?;
            self.pending_head.set(&block, state)?;
        }

        Ok(())
    }

    fn init_account(&mut self, acc: AccountData, state: &mut impl GenesisState<S>) -> Result<()> {
        let mut evm_db = self.get_db(state);
        evm_db.insert_account_info(
            acc.address,
            AccountInfo {
                balance: U256::ZERO,
                code_hash: acc.code_hash,
                nonce: 0,
                code: None,
            },
        );

        if !acc.code.is_empty() {
            evm_db.insert_code(acc.code_hash, acc.code.clone());
        };

        Ok(())
    }
}

fn init_block(config: &EvmGenesisConfig) -> Block {
    let header = alloy_consensus::Header {
        beneficiary: config.chain_spec.coinbase,
        // This will be set in finalize_hook or in the next begin_rollup_block_hook
        state_root: KECCAK_EMPTY,
        gas_limit: config.chain_spec.block_gas_limit,
        timestamp: config.genesis_timestamp,
        base_fee_per_gas: Some(config.initial_base_fee),
        ..Default::default()
    };

    Block {
        header,
        transactions: 0u64..0u64,
    }
}

fn init_spec(config: &EvmGenesisConfig) -> Result<Vec<(BlockNumber, SpecId)>> {
    let mut spec = config
        .chain_spec
        .hardforks
        .iter()
        .map(|&(k, v)| {
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
            if v == SpecId::CANCUN {
                panic!("Cancun is not supported");
            }

            (k, v)
        })
        .collect::<Vec<_>>();

    spec.sort_by(|a, b| a.0.cmp(&b.0));

    if spec.is_empty() {
        spec.push((0, SpecId::SHANGHAI));
    } else if spec[0].0 != 0u64 {
        panic!("EVM spec must start from block 0");
    };

    Ok(spec)
}

fn evm_chain_config(cfg: &EvmGenesisConfig, spec: Vec<(BlockNumber, SpecId)>) -> EvmRuntimeConfig {
    EvmRuntimeConfig {
        chain_spec: cfg.chain_spec.clone(),
        hardforks: spec,
    }
}
