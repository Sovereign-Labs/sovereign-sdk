use alloy_eips::BlockId;
use alloy_primitives::{Address, Bytes, TxKind, U64};
use alloy_rpc_types::TransactionRequest;
use sov_evm::{CallMessage, ChainSpecUpdate, Evm, EvmRuntimeConfigUpdate};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::ApiStateAccessor;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

use crate::helpers::{create_deploy_tx, create_transfer_tx, setup};
use crate::runtime::{RT, S};

const HISTORICAL_BLOCK: u64 = 1;
const ORIGINAL_TX_GAS_LIMIT: u64 = 30_000_000;
const UPDATED_TX_GAS_LIMIT: u64 = 20_000_000;

fn burn_gas_request(
    caller: Address,
    contract_addr: Address,
    iterations: u32,
    gas_limit: Option<u64>,
) -> TransactionRequest {
    TransactionRequest {
        from: Some(caller),
        to: Some(TxKind::Call(contract_addr)),
        input: LegacySimpleStorage::default().burn_gas(iterations).into(),
        gas: gas_limit,
        ..Default::default()
    }
}

fn historical_burn_gas_call_succeeds(
    evm: &Evm<S>,
    state: &mut ApiStateAccessor<S>,
    caller: Address,
    contract_addr: Address,
    iterations: u32,
    gas_limit: u64,
) -> bool {
    evm.eth_call(
        burn_gas_request(caller, contract_addr, iterations, Some(gas_limit)),
        Some(BlockId::number(HISTORICAL_BLOCK)),
        None,
        None,
        state,
    )
    .is_ok()
}

fn find_burn_gas_iterations_requiring_archival_tx_gas_cap(
    evm: &Evm<S>,
    state: &mut ApiStateAccessor<S>,
    caller: Address,
    contract_addr: Address,
) -> u32 {
    let mut low = 1u32;
    let mut high = 1u32;
    while historical_burn_gas_call_succeeds(
        evm,
        state,
        caller,
        contract_addr,
        high,
        UPDATED_TX_GAS_LIMIT,
    ) {
        low = high;
        high = high
            .checked_mul(2)
            .expect("burnGas iteration search overflowed");
        assert!(
            high <= 16_777_216,
            "could not find burnGas iterations that exceed the 20M gas cap"
        );
    }

    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if historical_burn_gas_call_succeeds(
            evm,
            state,
            caller,
            contract_addr,
            mid,
            UPDATED_TX_GAS_LIMIT,
        ) {
            low = mid;
        } else {
            high = mid;
        }
    }

    assert!(
        historical_burn_gas_call_succeeds(
            evm,
            state,
            caller,
            contract_addr,
            high,
            ORIGINAL_TX_GAS_LIMIT,
        ),
        "expected burnGas({high}) to stay below the original 30M tx gas limit"
    );
    assert!(
        !historical_burn_gas_call_succeeds(
            evm,
            state,
            caller,
            contract_addr,
            high,
            UPDATED_TX_GAS_LIMIT,
        ),
        "expected burnGas({high}) to exceed the updated 20M tx gas limit"
    );

    high
}

fn update_tx_gas_limit(runner: &mut TestRunner<RT, S>, admin: &TestUser<S>, new_tx_gas_limit: u64) {
    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: Some(ChainSpecUpdate {
                    new_limit_contract_code_size: None,
                    new_block_gas_limit: None,
                    new_tx_gas_limit: Some(new_tx_gas_limit),
                }),
                new_admin: None,
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                evm.cfg(state).unwrap().chain_spec.tx_gas_limit,
                Some(new_tx_gas_limit)
            );
        }),
    });
}

fn setup_archival_burn_gas_regression_case(
) -> (TestRunner<RT, S>, Address, TestUser<S>, Address, u32) {
    let (mut runner, account, _, admin) = setup();
    let contract = LegacySimpleStorage::default();
    let caller = account.address();
    let contract_addr = caller.create(0);
    runner.execute(create_deploy_tx(0, &contract, &account).tx);

    let iterations = runner.query_visible_state(|state| {
        find_burn_gas_iterations_requiring_archival_tx_gas_cap(
            &Evm::<S>::default(),
            state,
            caller,
            contract_addr,
        )
    });

    (runner, caller, admin, contract_addr, iterations)
}

fn query_historical_call_and_estimate(
    runner: &mut TestRunner<RT, S>,
    request: &TransactionRequest,
) -> (Bytes, U64) {
    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let output = evm
            .eth_call(
                request.clone(),
                Some(BlockId::number(HISTORICAL_BLOCK)),
                None,
                None,
                state,
            )
            .unwrap();
        let estimate = evm
            .eth_estimate_gas_helper(
                request.clone(),
                Some(BlockId::number(HISTORICAL_BLOCK)),
                None,
                None,
                state,
            )
            .unwrap();
        (output, estimate)
    })
}

#[test]
fn test_state_at_different_depth_is_accessible() {
    let (mut runner, from, to, _) = setup();

    let evm = Evm::<S>::default();
    for tx_idx in 0..=1 {
        let transfer_tx = create_transfer_tx(tx_idx, &from, &to, 1).tx;
        runner.execute(transfer_tx);
    }
    runner.query_visible_state(|state| {
        let mut balance = |block: Option<&str>| {
            evm.get_balance(
                to.address(),
                block.map(|s| s.parse::<BlockId>().unwrap()),
                state,
            )
            .unwrap()
        };
        assert_eq!(balance(None), 2);
        assert_eq!(balance(Some("latest")), 2);
        assert_eq!(balance(Some("pending")), 2);
        assert_eq!(balance(Some("0x00")), 0);
        assert_eq!(balance(Some("0x01")), 1);
        assert_eq!(balance(Some("0x02")), 2);
    });
}

#[test]
fn test_historical_eth_call_and_estimate_gas_keep_archival_tx_gas_limit_for_omitted_gas() {
    let (mut runner, caller, admin, contract_addr, iterations) =
        setup_archival_burn_gas_regression_case();
    let request = burn_gas_request(caller, contract_addr, iterations, None);

    let (before_output, before_estimate) =
        query_historical_call_and_estimate(&mut runner, &request);

    update_tx_gas_limit(&mut runner, &admin, UPDATED_TX_GAS_LIMIT);

    let (after_output, after_estimate) = query_historical_call_and_estimate(&mut runner, &request);

    assert_eq!(before_output, after_output);
    assert_eq!(before_estimate, after_estimate);
}

#[test]
fn test_historical_create_access_list_keeps_archival_tx_gas_limit_for_omitted_gas() {
    let (mut runner, caller, admin, contract_addr, iterations) =
        setup_archival_burn_gas_regression_case();
    let request = burn_gas_request(caller, contract_addr, iterations, None);

    let before = runner.query_visible_state(|state| {
        Evm::<S>::default()
            .eth_create_access_list(
                request.clone(),
                Some(BlockId::number(HISTORICAL_BLOCK)),
                state,
            )
            .unwrap()
    });
    assert!(
        before.error.is_none(),
        "precondition failed: historical eth_createAccessList should succeed before lowering the live tx gas limit"
    );

    update_tx_gas_limit(&mut runner, &admin, UPDATED_TX_GAS_LIMIT);

    let after = runner.query_visible_state(|state| {
        Evm::<S>::default()
            .eth_create_access_list(
                request.clone(),
                Some(BlockId::number(HISTORICAL_BLOCK)),
                state,
            )
            .unwrap()
    });

    assert_eq!(before.access_list, after.access_list);
    assert_eq!(before.gas_used, after.gas_used);
    assert_eq!(before.error, after.error);
}
