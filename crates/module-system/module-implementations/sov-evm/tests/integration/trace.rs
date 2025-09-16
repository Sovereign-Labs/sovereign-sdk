use crate::helpers::*;
use crate::runtime::S;
use alloy_primitives::{Bytes, U256};
use alloy_rpc_types_trace::geth::{
    CallFrame, GethDebugBuiltInTracerType, GethDebugTracingOptions, GethTrace,
};
use sov_evm::Evm;
use sov_test_utils::{BatchTestCase, SimpleStorageContract};

#[test]
fn test_tracing() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);

    let mut nonce = 0;
    runner.execute(create_deploy_tx(nonce, &contract, &account).tx);
    nonce += 1;

    let mut tx_hash = None;
    for _batch_id in 1..10 {
        let mut txs = vec![];

        for _tx_id in 1..10 {
            let tx = create_set_arg_tx(0, nonce, &contract, contract_addr, &account);
            txs.push(tx.tx);
            if nonce == 48 {
                tx_hash = Some(tx.hash);
            }
            nonce += 1;
        }

        runner.execute_batch(BatchTestCase {
            input: txs.into(),
            assert: Box::new(move |_, _| {}),
        });
    }

    let evm = Evm::<S>::default();
    runner.query_state(|state| {
        let opts = GethDebugTracingOptions::new_tracer(GethDebugBuiltInTracerType::CallTracer);
        let trace = evm
            .debug_trace_transaction(tx_hash.unwrap(), Some(opts), state)
            .unwrap();
        assert_eq!(
            trace,
            GethTrace::CallTracer(CallFrame {
                from: account.address(),
                to: Some(contract_addr),
                typ: "CALL".into(),
                input: "60fe47b10000000000000000000000000000000000000000000000000000000000000000"
                    .parse::<Bytes>()
                    .unwrap(),
                value: Some(U256::ZERO),
                gas: U256::from(1_000_000),
                gas_used: U256::from(5_229),
                ..Default::default()
            })
        );
    });
}
