use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{Address, B256, U256};
use alloy_primitives::{BlockNumber, Bytes};
use anyhow::Result;
use revm::primitives::hardfork::SpecId;
use revm::state::AccountInfo;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::config_gas_token_id;
use sov_modules_api::{Amount, GenesisState, Module, Spec};

use crate::evm::db_init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::evm::EvmChainConfig;
use crate::{to_rollup_address, Evm, EvmConfig};

/// Evm account.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct AccountData {
    /// Account address.
    pub address: Address,
    /// Account balance.
    pub balance: U256,
    /// Code hash.
    pub code_hash: B256,
    /// Smart contract code.
    pub code: Bytes,
    /// Account nonce.
    pub nonce: u64,
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
        for acc in config.data.clone() {
            self.init_account(acc, state)?;
        }

        let spec = init_spec(config)?;
        let chain_cfg = evm_chain_config(config, spec);
        let block = init_block(config);

        self.cfg.set(&chain_cfg, state)?;
        self.head.set(&block, state)?;
        #[cfg(feature = "native")]
        self.pending_head.set(&block, state)?;

        Ok(())
    }

    fn init_account(
        &mut self,
        mut acc: AccountData,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        let rollup_address: <S as Spec>::Address = to_rollup_address::<S>(acc.address);
        let bank_balance =
            self.bank_module
                .get_balance_of(&rollup_address, config_gas_token_id(), state)?;

        assert!(
            !(acc.balance != U256::ZERO && bank_balance.is_some()),
            "EVM account balance can only be set from one genesis config to avoid conflicts. 
                Choose either the Bank or the EVM module genesis config."
        );

        if acc.balance != U256::ZERO {
            self.bank_module.override_gas_balance(
                Amount::new(acc.balance.try_into().unwrap()),
                &rollup_address,
                state,
            )?;
            acc.balance = U256::ZERO;
        }

        let mut evm_db = self.get_db(state);
        evm_db.insert_account_info(
            acc.address,
            AccountInfo {
                balance: acc.balance,
                code_hash: acc.code_hash,
                nonce: acc.nonce,
                code: None,
            },
        );

        if !acc.code.is_empty() {
            evm_db.insert_code(acc.code_hash, acc.code.clone());
        };

        Ok(())
    }
}

fn init_block(config: &EvmConfig) -> Block {
    let header = alloy_consensus::Header {
        beneficiary: config.coinbase,
        // This will be set in finalize_hook or in the next begin_rollup_block_hook
        state_root: KECCAK_EMPTY,
        gas_limit: config.block_gas_limit,
        timestamp: config.genesis_timestamp,
        base_fee_per_gas: Some(config.starting_base_fee),
        ..Default::default()
    };

    Block {
        header,
        transactions: 0u64..0u64,
    }
}

fn init_spec(config: &EvmConfig) -> Result<Vec<(BlockNumber, SpecId)>> {
    let mut spec = config
        .spec
        .iter()
        .map(|(k, v)| {
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
            if *v == SpecId::CANCUN {
                panic!("Cancun is not supported");
            }

            (*k, *v)
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

fn evm_chain_config(cfg: &EvmConfig, spec: Vec<(BlockNumber, SpecId)>) -> EvmChainConfig {
    EvmChainConfig {
        spec,
        chain_id: cfg.chain_id,
        limit_contract_code_size: cfg.limit_contract_code_size,
        coinbase: cfg.coinbase,
        block_gas_limit: cfg.block_gas_limit,
        block_timestamp_delta: cfg.block_timestamp_delta,
        base_fee_params: cfg.base_fee_params,
    }
}
