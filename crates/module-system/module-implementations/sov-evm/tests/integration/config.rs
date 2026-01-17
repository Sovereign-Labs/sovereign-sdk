use std::str::FromStr;

use alloy_primitives::Address;

use crate::helpers::setup;
use crate::runtime::{RT, S};
use sov_address::EthereumAddress;
use sov_address::MultiAddress;
use sov_evm::BorshSpecId;
use sov_evm::CallMessage;
use sov_evm::ChainSpecUpdate;
use sov_evm::ContractCreationPolicy;
use sov_evm::ContractCreationPolicyUpdate;
use sov_evm::Evm;
use sov_evm::EvmRuntimeConfigUpdate;
use sov_evm::SpecId;
use sov_modules_api::HexString;
use sov_modules_api::SafeVec;
use sov_modules_api::TxEffect;
use sov_test_utils::AsUser;
use sov_test_utils::TransactionTestCase;

#[test]
fn test_empty_config_update_is_noop() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();
    // Send an empty config update and verify that the config is unchanged
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(evm.admin(state).unwrap(), admin.address());
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(cfg.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
            assert_eq!(cfg.chain_spec.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(cfg.chain_spec.coinbase, Address::ZERO);
            assert_eq!(cfg.chain_spec.block_gas_limit, 1_000_000_000);
            assert_eq!(cfg.chain_spec.tx_gas_limit, Some(30_000_000));
            assert_eq!(cfg.chain_spec.limit_contract_code_size, None);
        }),
    });
}

#[test]
fn test_update_hardfork() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();
    let admin_address = admin.address();
    // Update the hardfork to Prague. Verify that only the hardforks list has changed
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: Some((100, BorshSpecId(SpecId::PRAGUE))),
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(evm.admin(state).unwrap(), admin_address);
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(
                cfg.hardforks,
                vec![(0, SpecId::CANCUN), (100, SpecId::PRAGUE)]
            );
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
            assert_eq!(
                cfg.chain_spec.hardforks,
                vec![(0, SpecId::CANCUN), (100, SpecId::PRAGUE)]
            );
            assert_eq!(cfg.chain_spec.coinbase, Address::ZERO);
            assert_eq!(cfg.chain_spec.block_gas_limit, 1_000_000_000);
            assert_eq!(cfg.chain_spec.tx_gas_limit, Some(30_000_000));
            assert_eq!(cfg.chain_spec.limit_contract_code_size, None);
        }),
    });

    // Test that a hardfork with a spec ID less than or equal to the current spec ID is rejected
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig (
            EvmRuntimeConfigUpdate {
                new_hardfork: Some((105, BorshSpecId(SpecId::PRAGUE))),
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, _state| {
			let TxEffect::Reverted(reverted) = ctx.tx_receipt else {
				panic!("Expected a reverted transaction");
			};
			assert!(reverted.reason.to_string().contains("Hardfork spec ID must be greater than the current spec ID"), "Expected a revert reason containing 'Hardfork spec ID must be greater than the current spec ID'");
        }),
    });

    // Test that a hardfork with a activation block number less than or equal to the activation block of the newest hardfork is rejected
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig (
            EvmRuntimeConfigUpdate {
                new_hardfork: Some((100, BorshSpecId(SpecId::OSAKA))),
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, _state| {
			let TxEffect::Reverted(reverted) = ctx.tx_receipt else {
				panic!("Expected a reverted transaction");
			};
			assert!(reverted.reason.to_string().contains("Hardfork activation block number must be greater than the activation block of the newest hardfork"), "Expected a revert reason containing 'Hardfork activation block number must be greater than the activation block of the newest hardfork'");
        }),
    });
}

#[test]
fn test_update_contract_creation_policy() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();
    let admin_address = admin.address();
    let new_admin_address = "0x0123456789012345678901234567890123456789";
    let new_admin_address_alloy = Address::from_str(new_admin_address).unwrap();
    let new_admin_address_hex_string = HexString::from_str(new_admin_address).unwrap();
    let another_address = "0x0123456789012345678901234567890123456788";
    let another_address_alloy = Address::from_str(another_address).unwrap();
    let another_address_hex_string = HexString::from_str(another_address).unwrap();

    // Switch to an allowlist with the new admin addresss. Note that removal is a no-op, since "another_address" is not on the allowlist.
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Allowlist {
                    add: SafeVec::try_from(vec![new_admin_address_hex_string]).unwrap(),
                    remove: SafeVec::try_from(vec![another_address_hex_string]).unwrap(),
                }),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(evm.admin(state).unwrap(), admin_address);
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(cfg.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Allowlist(
                    vec![new_admin_address_alloy].into_iter().collect()
                )
            );
            assert_eq!(cfg.chain_spec.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(cfg.chain_spec.coinbase, Address::ZERO);
            assert_eq!(cfg.chain_spec.block_gas_limit, 1_000_000_000);
            assert_eq!(cfg.chain_spec.tx_gas_limit, Some(30_000_000));
            assert_eq!(cfg.chain_spec.limit_contract_code_size, None);
        }),
    });

    // Add another address to the allowlist and remove the existing "new_admin_address"
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Allowlist {
                    add: SafeVec::try_from(vec![another_address_hex_string]).unwrap(),
                    remove: SafeVec::try_from(vec![new_admin_address_hex_string]).unwrap(),
                }),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |_ctx, state| {
            let cfg = Evm::<S>::default().cfg(state).unwrap();
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Allowlist(
                    vec![another_address_alloy].into_iter().collect()
                )
            );
        }),
    });

    // Delete the remaining address from the allowlist
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Allowlist {
                    add: SafeVec::try_from(vec![]).unwrap(),
                    remove: SafeVec::try_from(vec![another_address_hex_string]).unwrap(),
                }),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |_ctx, state| {
            let cfg = Evm::<S>::default().cfg(state).unwrap();
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Allowlist(vec![].into_iter().collect())
            );
        }),
    });

    // Switch to everyone
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Everyone),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |_ctx, state| {
            let cfg = Evm::<S>::default().cfg(state).unwrap();
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
        }),
    });
}

#[test]
fn test_update_admin() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();
    let new_admin_address = "0x0123456789012345678901234567890123456789";
    let new_admin_address_alloy =
        MultiAddress::<EthereumAddress>::from_str(new_admin_address).unwrap();

    // Set the new admin address
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: Some(new_admin_address_alloy),
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(evm.admin(state).unwrap(), new_admin_address_alloy);
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(cfg.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
            assert_eq!(cfg.chain_spec.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(cfg.chain_spec.coinbase, Address::ZERO);
            assert_eq!(cfg.chain_spec.block_gas_limit, 1_000_000_000);
            assert_eq!(cfg.chain_spec.tx_gas_limit, Some(30_000_000));
            assert_eq!(cfg.chain_spec.limit_contract_code_size, None);
        }),
    });

    // Assert that the old admin can no longer update the config
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig (
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Everyone),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, _state| {
			let TxEffect::Reverted(reverted) = ctx.tx_receipt else {
				panic!("Expected a reverted transaction");
			};
			assert!(reverted.reason.to_string().contains("Only the admin can update the runtime configuration"), "Expected a revert reason containing 'Only the admin can update the runtime configuration' got {}", reverted.reason);
        }),
    });
}

#[test]
fn test_update_chain_spec() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();
    let admin_address = admin.address();

    // Set the contract code size limit and assert that all other fields are unchanged
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: Some(ChainSpecUpdate {
                    new_limit_contract_code_size: Some(1000),
                    new_block_gas_limit: None,
                    new_tx_gas_limit: None,
                }),
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(evm.admin(state).unwrap(), admin_address);
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(cfg.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
            assert_eq!(cfg.chain_spec.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(cfg.chain_spec.coinbase, Address::ZERO);
            assert_eq!(cfg.chain_spec.block_gas_limit, 1_000_000_000);
            assert_eq!(cfg.chain_spec.tx_gas_limit, Some(30_000_000));
            assert_eq!(cfg.chain_spec.limit_contract_code_size, Some(1000));
        }),
    });

    // Set the block gas limit and assert that all other fields are unchanged
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: Some(ChainSpecUpdate {
                    new_limit_contract_code_size: None,
                    new_block_gas_limit: Some(50_000_000),
                    new_tx_gas_limit: None,
                }),
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                Evm::<S>::default()
                    .cfg(state)
                    .unwrap()
                    .chain_spec
                    .block_gas_limit,
                50_000_000
            );
        }),
    });

    // Set the tx gas limit and assert that all other fields are unchanged
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: Some(ChainSpecUpdate {
                    new_limit_contract_code_size: None,
                    new_block_gas_limit: Some(50_000_000),
                    new_tx_gas_limit: Some(20_000_000),
                }),
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                Evm::<S>::default()
                    .cfg(state)
                    .unwrap()
                    .chain_spec
                    .tx_gas_limit,
                Some(20_000_000)
            );
        }),
    });
}

#[test]
fn test_disable_max_fee_check() {
    let (mut runner, _, _, admin) = setup();

    let evm = Evm::<S>::default();

    // Verify that disable_max_fee_check is false by default
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Everyone),
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert!(
                !evm.is_max_fee_check_disabled(state).unwrap(),
                "disable_max_fee_check should be false by default"
            );
        }),
    });

    let evm = Evm::<S>::default();
    let admin_address = admin.address();

    // Send an empty config update (all fields None) and verify that disable_max_fee_check is set to true
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            // Verify that disable_max_fee_check is now true
            assert!(
                evm.is_max_fee_check_disabled(state).unwrap(),
                "disable_max_fee_check should be true after empty config update"
            );
            // Verify that other config fields are unchanged
            assert_eq!(evm.admin(state).unwrap(), admin_address);
            let cfg = evm.cfg(state).unwrap();
            assert_eq!(cfg.hardforks, vec![(0, SpecId::CANCUN)]);
            assert_eq!(
                cfg.contract_creation_policy,
                ContractCreationPolicy::Everyone
            );
        }),
    });
}
