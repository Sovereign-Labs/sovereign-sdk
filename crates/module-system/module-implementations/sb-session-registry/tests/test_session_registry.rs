#![cfg(test)]

use sov_modules_api::Spec;
use sov_test_utils::{generate_optimistic_runtime, TestSpec};

use sb_session_registry::{CallMessage, RegistryConfig, SessionRegistry};

mod common;
use common::{DexCallMessage, DexConfig, TestDex};

type S = TestSpec;

generate_optimistic_runtime!(
    TestRuntime <=
    session_registry: SessionRegistry<S>,
    test_dex: TestDex<S>
);

use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

pub struct TestData<S: Spec> {
    pub owner: TestUser<S>,
    pub manager: TestUser<S>,
    pub signer: TestUser<S>,
    pub wallet: TestUser<S>,
    pub wallet2: TestUser<S>,
}

pub fn setup() -> (TestData<S>, TestRunner<TestRuntime<S>, S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(5);

    let mut users = genesis_config.additional_accounts().to_vec();
    let wallet2 = users.pop().expect("second wallet user");
    let wallet = users.pop().expect("wallet user");
    let signer = users.pop().expect("signer user");
    let manager = users.pop().expect("manager user");
    let owner = users.pop().expect("owner user");

    let test_data = TestData {
        owner,
        manager,
        signer,
        wallet,
        wallet2,
    };

    let registry_config = RegistryConfig::<S> {
        owner: test_data.owner.address(),
        manager: test_data.manager.address(),
        enforcement_enabled: true,
        expiry_offset: 0,
    };

    let dex_config = DexConfig {};

    let genesis =
        GenesisConfig::from_minimal_config(genesis_config.into(), registry_config, dex_config);

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (test_data, runner)
}

//
// TEST 1 – basic signer / session lifecycle
//
// - DEX enforces session active for wallet (should fail: no signer, no session)
// - Manager designates a session signer
// - Signer sets session for wallet with future expiry
// - DEX enforces session active and present (should succeed)
// - Signer clears session for wallet with ttl 0
// - DEX enforces session present and active (both should fail)
// - Owner attempts to set session signer (should fail: only manager allowed)
// - Owner attempts to set session (should fail: only signer allowed)
//
#[test]
fn test_1() {
    let (test_data, mut runner) = setup();

    let owner = &test_data.owner;
    let manager = &test_data.manager;
    let signer = &test_data.signer;
    let wallet = &test_data.wallet;

    let signer_addr = signer.address().clone();
    let wallet_addr = wallet.address().clone();

    // DEX enforces session active (should fail: no signer, no session)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionActive should fail when no signer and no session are configured"
            );
        }),
    });

    // Manager sets a session signer
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSessionSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSessionSigner should succeed for manager"
            );
        }),
    });

    // Signer sets session for wallet, expiry in the future
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSession {
                wallet: wallet_addr.clone(),
                expires_at: 2764177788,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSession should succeed for authorized session signer"
            );
        }),
    });

    // DEX enforces that session is active
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionActive should succeed for valid session"
            );
        }),
    });

    // DEX enforces session present
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionPresent should succeed for existing session"
            );
        }),
    });

    // Signer clears session for wallet with expiry 0
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSession {
                wallet: wallet_addr.clone(),
                expires_at: 0,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSession with ttl=0 should succeed and clear the session"
            );
        }),
    });

    // DEX enforces session present (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionPresent should fail once session has been cleared"
            );
        }),
    });

    // DEX enforces session active (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionActive should fail once session has been cleared"
            );
        }),
    });

    // Owner sets session signer (should fail because only manager is allowed)
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSessionSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetSessionSigner should fail when called by owner (not manager)"
            );
        }),
    });

    // Owner sets session (should fail because only signer allowed)
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSession {
                wallet: wallet_addr.clone(),
                expires_at: 2764177788,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetSession should fail when called by non-signer"
            );
        }),
    });
}

//
// TEST 2 – batch sessions and per-wallet isolation
//
// - Manager sets a session signer
// - Signer sets batch session for wallet and wallet2 with future expiries
// - DEX enforces session active for both wallets (should succeed)
// - Signer clears session for wallet with ttl 0
// - DEX enforces session present / active for wallet (should fail)
// - DEX enforces session present / active for wallet2 (should succeed)
//
#[test]
fn test_2() {
    let (test_data, mut runner) = setup();

    let manager = &test_data.manager;
    let signer = &test_data.signer;
    let wallet = &test_data.wallet;
    let wallet2 = &test_data.wallet2;

    let signer_addr = signer.address().clone();
    let wallet_addr = wallet.address().clone();
    let wallet2_addr = wallet2.address().clone();

    // Manager sets session signer
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSessionSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSessionSigner should succeed for manager"
            );
        }),
    });

    // Signer sets batch session for wallet and wallet2 with expiry in the future
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSessionBatch {
                wallets: vec![wallet_addr.clone(), wallet2_addr.clone()],
                expiries: vec![2764177788, 2764177788],
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSessionBatch should succeed for authorized signer"
            );
        }),
    });

    // DEX enforces session active for both wallets (should succeed)
    for target_wallet in [wallet_addr.clone(), wallet2_addr.clone()] {
        runner.execute_transaction(TransactionTestCase {
            input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
                DexCallMessage::EnforceSessionActive {
                    wallet: target_wallet,
                },
            ),
            assert: Box::new(|result, _state| {
                assert!(
                    result.tx_receipt.is_successful(),
                    "EnforceSessionActive should succeed for all wallets in the batch"
                );
            }),
        });
    }

    // Signer clears session for wallet with ttl 0
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSession {
                wallet: wallet_addr.clone(),
                expires_at: 0,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSession with ttl=0 should clear only wallet's session"
            );
        }),
    });

    // DEX enforces session present for wallet (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionPresent should fail for wallet after its session is cleared"
            );
        }),
    });

    // DEX enforces session present for wallet2 (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet2_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionPresent should succeed for wallet2"
            );
        }),
    });

    // DEX enforces session active for wallet (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionActive should fail for wallet after its session is cleared"
            );
        }),
    });

    // DEX enforces session active for wallet2 (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet2_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionActive should succeed for wallet2"
            );
        }),
    });
}

//
// TEST 3 – per-wallet bypass and global enforcement flag
//
// - Manager sets bypass for wallet to true (no prior session)
// - DEX enforces session present and active (should succeed via bypass)
// - Manager sets bypass for wallet to false (entry removed if only bypass w/ expiry_ts=0)
// - DEX enforces session present and active (should fail: no session, no bypass)
// - Manager sets enforcement_enabled to false
// - DEX enforces session present and active (should now be no-op and succeed)
// - Manager uses direct call endpoints EnforceSessionActive/Present (succeed)
// - Owner changes manager to owner address
// - New manager (owner) can now set session signer successfully
//
#[test]
fn test_3() {
    let (test_data, mut runner) = setup();

    let owner = &test_data.owner;
    let manager = &test_data.manager;
    let signer = &test_data.signer;
    let wallet = &test_data.wallet;

    let owner_addr = owner.address().clone();
    let signer_addr = signer.address().clone();
    let wallet_addr = wallet.address().clone();

    // Manager sets bypass for wallet to true (creates a pure-bypass session if none exists)
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetBypass {
                wallet: wallet_addr.clone(),
                bypass: true,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBypass(true) should succeed for manager"
            );
        }),
    });

    // DEX enforces session present (should succeed via bypass)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionPresent should succeed with wallet-level bypass"
            );
        }),
    });

    // DEX enforces session active (should succeed via bypass)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionActive should succeed with wallet-level bypass"
            );
        }),
    });

    // Manager sets bypass for wallet to false
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetBypass {
                wallet: wallet_addr.clone(),
                bypass: false,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBypass(false) should succeed and remove the pure-bypass session"
            );
        }),
    });

    // DEX enforces session present (should now fail: no session, no bypass)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionPresent should fail once bypass is removed and no session exists"
            );
        }),
    });

    // DEX enforces session active (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceSessionActive should fail once bypass is removed and no session exists"
            );
        }),
    });

    // Owner disables global enforcement
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetEnforcementEnabled { enabled: false },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetEnforcementEnabled(false) should succeed for owner"
            );
        }),
    });

    // With enforcement disabled, DEX checks should succeed even without a session
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionActive should succeed when enforcement is disabled"
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceSessionPresent should succeed when enforcement is disabled"
            );
        }),
    });

    // Direct call endpoints should behave the same: no-op under disabled enforcement
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::EnforceSessionActive {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Direct EnforceSessionActive should succeed when enforcement is disabled"
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::EnforceSessionPresent {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Direct EnforceSessionPresent should succeed when enforcement is disabled"
            );
        }),
    });

    // Owner changes manager to owner address
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetManager {
                new_manager: owner_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetManager should succeed when called by owner"
            );
        }),
    });

    // New manager (owner) can now set session signer successfully
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, SessionRegistry<S>>(
            CallMessage::SetSessionSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetSessionSigner should succeed once owner has become manager"
            );
        }),
    });
}
