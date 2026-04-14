use std::str::FromStr;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
use sov_address::{EthereumAddress, MultiAddress};
use sov_evm::{AccountData, ContractCreationPolicy, EvmChainSpec, EvmGenesisConfig, SpecId};
use sov_modules_api::{Amount, CryptoSpec, Spec};
use sov_paymaster::{
    AuthorizedSequencers, PayeePolicy, PayerGenesisConfig, PaymasterConfig,
    PaymasterPolicyInitializer, SafeVec,
};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::TestUser;

use crate::common::constants::{
    AFFORDABILITY_SIGNER_PRIV_KEY, DEFAULT_EVM_BALANCE, DEFAULT_FUNDED_EVM_ACCOUNTS,
    PAYER_SOV_BANK_BALANCE, PAYMASTER_SIGNER_PRIV_KEY, SECONDARY_SENDER_PRIV_KEY, SENDER_PRIV_KEY,
};
use crate::runtime::{EvmTestSpec, GenesisConfig};

/// A constructed genesis with metadata the test rollup needs to start.
pub struct TestGenesis {
    pub genesis: GenesisConfig<EvmTestSpec>,
    pub seq_da_address: <<EvmTestSpec as Spec>::Da as sov_rollup_interface::da::DaSpec>::Address,
    /// The paymaster (sov user) registered in this genesis, if any.
    pub paymaster: Option<TestUser<EvmTestSpec>>,
    /// Private key of the admin used for the EVM genesis config (sequencer's user).
    pub admin_private_key: <<EvmTestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
}

/// Constructs a default genesis matching `chain_state_zk.json` + `bank.json` + `evm.json`
/// from the integration-tests test-data directory: pre-funds the standard hardhat addresses
/// and uses an `Everyone` contract creation policy so deployments work in most tests.
pub fn default_genesis() -> TestGenesis {
    build_genesis(GenesisOptions::default())
}

/// Constructs a genesis with a paymaster pre-registered with an `Allow` policy for everyone.
/// Mirrors `paymaster_with_payer.json`.
pub fn paymaster_with_payer_genesis() -> TestGenesis {
    build_genesis(GenesisOptions {
        paymaster_kind: PaymasterKind::AllowAll,
        ..GenesisOptions::default()
    })
}

/// Constructs a genesis with a paymaster pre-registered with a `Deny`-by-default policy
/// that explicitly allows `SENDER_PRIV_KEY` and `PAYMASTER_SIGNER_PRIV_KEY`.
/// Mirrors `paymaster_selective.json`.
pub fn paymaster_selective_genesis() -> TestGenesis {
    let sender_addr = signer_address(SENDER_PRIV_KEY);
    let paymaster_signer_addr = signer_address(PAYMASTER_SIGNER_PRIV_KEY);
    build_genesis(GenesisOptions {
        paymaster_kind: PaymasterKind::DenyExcept(vec![sender_addr, paymaster_signer_addr]),
        ..GenesisOptions::default()
    })
}

#[derive(Clone)]
pub struct GenesisOptions {
    /// Extra EVM addresses to pre-fund with `DEFAULT_EVM_BALANCE` (in addition to
    /// `DEFAULT_FUNDED_EVM_ACCOUNTS`).
    pub extra_funded_evm_accounts: Vec<Address>,
    /// Override the contract creation policy. Defaults to `Everyone`.
    pub contract_creation_policy: Option<ContractCreationPolicy>,
    /// Initial base fee for the first EVM block. Defaults to 10 (mirrors `evm.json`).
    pub initial_base_fee: u64,
    pub paymaster_kind: PaymasterKind,
}

impl Default for GenesisOptions {
    fn default() -> Self {
        Self {
            extra_funded_evm_accounts: Vec::new(),
            contract_creation_policy: None,
            initial_base_fee: 10,
            paymaster_kind: PaymasterKind::None,
        }
    }
}

#[derive(Default, Clone)]
pub enum PaymasterKind {
    /// No paymaster registered.
    #[default]
    None,
    /// Paymaster covers all senders unconditionally.
    AllowAll,
    /// Paymaster denies by default, except for the explicitly listed EVM addresses.
    DenyExcept(Vec<Address>),
}

fn signer_address(priv_key: &str) -> Address {
    let signer: PrivateKeySigner = priv_key.parse().expect("valid hardhat private key");
    signer.address()
}

pub fn build_genesis(opts: GenesisOptions) -> TestGenesis {
    let mut hl_genesis = HighLevelOptimisticGenesisConfig::<EvmTestSpec>::generate();

    // Generate a paymaster sov-user with `PAYER_SOV_BANK_BALANCE` and add to additional_accounts
    // so the bank knows about it.
    let paymaster_user = match opts.paymaster_kind {
        PaymasterKind::None => None,
        PaymasterKind::AllowAll | PaymasterKind::DenyExcept(_) => {
            let user = TestUser::generate(Amount::new(PAYER_SOV_BANK_BALANCE));
            hl_genesis.additional_accounts_mut().push(user.clone());
            Some(user)
        }
    };

    // The sequencer's DA address — needed both for paymaster `sequencers_to_register`
    // and for routing blobs at runtime.
    let seq_da_address = hl_genesis.initial_sequencer.da_address;

    let admin_address = hl_genesis.initial_sequencer.user_info.address();
    let admin_private_key = hl_genesis.initial_sequencer.user_info.private_key.clone();

    // Mirror `examples/test-data/genesis/integration-tests/evm.json`: tests cross-check
    // `gas_used_ratio` against the block header, so the block gas limit must match.
    let evm_chain_spec = EvmChainSpec {
        block_gas_limit: 100_000_000_000,
        tx_gas_limit: Some(30_000_000),
        hardforks: vec![(0, SpecId::CANCUN)],
        ..EvmChainSpec::default()
    };

    let evm_config = EvmGenesisConfig::<EvmTestSpec> {
        accounts: default_funded_evm_accounts(&opts)
            .into_iter()
            .map(AccountData::empty_with_address)
            .collect(),
        initial_base_fee: opts.initial_base_fee,
        genesis_timestamp: 0,
        chain_spec: evm_chain_spec,
        contract_creation_policy: opts
            .contract_creation_policy
            .clone()
            .unwrap_or(ContractCreationPolicy::Everyone),
        admin: admin_address,
    };

    let paymaster_config = match (&opts.paymaster_kind, paymaster_user.as_ref()) {
        (PaymasterKind::None, _) => PaymasterConfig::default(),
        (PaymasterKind::AllowAll, Some(user)) => PaymasterConfig {
            payers: [PayerGenesisConfig {
                payer_address: user.address(),
                policy: PaymasterPolicyInitializer {
                    default_payee_policy: PayeePolicy::Allow {
                        max_fee: None,
                        gas_limit: None,
                        max_gas_price: None,
                        transaction_limit: None,
                    },
                    payees: SafeVec::new(),
                    authorized_sequencers: AuthorizedSequencers::All,
                    authorized_updaters: SafeVec::new(),
                },
                sequencers_to_register: [seq_da_address]
                    .as_ref()
                    .try_into()
                    .expect("single sequencer fits in SafeVec"),
            }]
            .as_ref()
            .try_into()
            .expect("single payer fits in SafeVec"),
        },
        (PaymasterKind::DenyExcept(allowed), Some(user)) => {
            let payees: Vec<_> = allowed
                .iter()
                .map(|addr| {
                    (
                        MultiAddress::Vm(EthereumAddress::from(*addr)),
                        PayeePolicy::Allow {
                            max_fee: None,
                            gas_limit: None,
                            max_gas_price: None,
                            transaction_limit: None,
                        },
                    )
                })
                .collect();
            let payees: SafeVec<_, _> = payees
                .as_slice()
                .try_into()
                .expect("payee list within bounds");
            PaymasterConfig {
                payers: [PayerGenesisConfig {
                    payer_address: user.address(),
                    policy: PaymasterPolicyInitializer {
                        default_payee_policy: PayeePolicy::Deny,
                        payees,
                        authorized_sequencers: AuthorizedSequencers::All,
                        authorized_updaters: SafeVec::new(),
                    },
                    sequencers_to_register: [seq_da_address]
                        .as_ref()
                        .try_into()
                        .expect("single sequencer fits in SafeVec"),
                }]
                .as_ref()
                .try_into()
                .expect("single payer fits in SafeVec"),
            }
        }
        (PaymasterKind::AllowAll | PaymasterKind::DenyExcept(_), None) => {
            unreachable!("paymaster_user is constructed when kind != None")
        }
    };

    let mut genesis: GenesisConfig<EvmTestSpec> =
        GenesisConfig::from_minimal_config(hl_genesis.into(), evm_config, paymaster_config);

    // Top up bank balances for the default EVM accounts (mirrors bank.json).
    if let Some(token_cfg) = genesis.bank.gas_token_config.as_mut() {
        for addr in default_funded_evm_accounts(&opts) {
            token_cfg.address_and_balances.push((
                MultiAddress::Vm(EthereumAddress::from(addr)),
                Amount::new(DEFAULT_EVM_BALANCE),
            ));
        }
    }

    TestGenesis {
        genesis,
        seq_da_address,
        paymaster: paymaster_user,
        admin_private_key,
    }
}

fn default_funded_evm_accounts(opts: &GenesisOptions) -> Vec<Address> {
    let mut out: Vec<Address> = DEFAULT_FUNDED_EVM_ACCOUNTS
        .iter()
        .map(|s| Address::from_str(s).expect("constant default-funded address parses"))
        .collect();
    out.extend(opts.extra_funded_evm_accounts.iter().copied());
    out
}

/// Suppress unused warnings for items that some tests may not call.
#[allow(dead_code)]
fn _unused_imports_helper() {
    let _ = AFFORDABILITY_SIGNER_PRIV_KEY;
    let _ = SECONDARY_SENDER_PRIV_KEY;
}
