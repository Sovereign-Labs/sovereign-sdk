use alloy_primitives::TxKind;
use alloy_rpc_types::TransactionRequest;
use sov_evm::build_request_preflight_auth;
use sov_modules_api::capabilities::UniquenessData;

use crate::helpers::{create_transfer_tx, setup};
use crate::runtime::S;

fn request_preflight(
    from: alloy_primitives::Address,
    to: alloy_primitives::Address,
) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        gas: Some(21_000),
        max_fee_per_gas: Some(100),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    }
}

#[test]
fn request_preflight_auth_uses_current_nonce_when_request_nonce_omitted() {
    let (mut runner, account, recipient, _) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);

    let request = request_preflight(account.address(), recipient.address());
    runner.query_visible_state(move |state| {
        let mut auth_state = state.clone_without_local_writes().to_provable_reader();
        let (_, auth_data) = build_request_preflight_auth::<_, S>(&request, &mut auth_state)
            .expect("request preflight auth should succeed");

        assert_eq!(auth_data.uniqueness, UniquenessData::Nonce(1));
    });
}

#[test]
fn request_preflight_auth_preserves_explicit_nonce() {
    let (runner, account, recipient, _) = setup();
    let mut request = request_preflight(account.address(), recipient.address());
    request.nonce = Some(7);

    runner.query_visible_state(move |state| {
        let mut auth_state = state.clone_without_local_writes().to_provable_reader();
        let (_, auth_data) = build_request_preflight_auth::<_, S>(&request, &mut auth_state)
            .expect("request preflight auth should succeed");

        assert_eq!(auth_data.uniqueness, UniquenessData::Nonce(7));
    });
}
