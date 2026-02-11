use sov_hyperlane_integration::warp::TokenKind;
use sov_hyperlane_integration::{Ism, Recipient};
use sov_modules_api::{HexHash, HexString, SafeVec};

use crate::setup::{register_warp_route_with_ism_and_token_source, setup, SetupParams};

/// When a warp route has a dedicated ISM, `SolanaRegistration::ism(route_id)` should
/// return that per-route ISM rather than the module-level default.
#[test]
fn test_ism_returns_warp_route_ism_over_default() {
    let SetupParams {
        mut runner, admin, ..
    } = setup();

    // The default ISM from genesis is AlwaysTrust.
    // Register a warp route with a *different* ISM (MessageIdMultisig).
    let route_ism = Ism::MessageIdMultisig {
        validators: SafeVec::try_from(vec![HexString([0xAB; 20])]).unwrap(),
        threshold: 1,
    };

    let route_id = register_warp_route_with_ism_and_token_source(
        &mut runner,
        &admin,
        route_ism.clone(),
        TokenKind::Native,
    );

    runner.query_state(|state| {
        let module = sov_hyperlane_register_module::SolanaRegistration::default();

        // Querying with the route ID should return the warp route's ISM
        let resolved = module.ism(&route_id, state).unwrap();
        assert_eq!(
            resolved,
            Some(route_ism.clone()),
            "ism() should return the warp route's per-route ISM, not the default"
        );

        // Querying with an unknown recipient should fall back to the default ISM
        let unknown_recipient = HexHash::new([0xFF; 32]);
        let fallback = module.ism(&unknown_recipient, state).unwrap();
        assert_eq!(
            fallback,
            Some(Ism::AlwaysTrust),
            "ism() should fall back to the default ISM for unknown recipients"
        );
    });
}
