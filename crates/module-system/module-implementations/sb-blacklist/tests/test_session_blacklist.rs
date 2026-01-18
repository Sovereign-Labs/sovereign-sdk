#![cfg(test)]

use sov_modules_api::Spec;
use sov_test_utils::{generate_optimistic_runtime, TestSpec};

use sb_blacklist::{Blacklist, BlacklistConfig, CallMessage};

mod common;
use common::{DexCallMessage, DexConfig, TestDex};

type S = TestSpec;

generate_optimistic_runtime!(
    TestRuntime <=
    blacklist: Blacklist<S>,
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

    let blacklist_config = BlacklistConfig::<S> {
        owner: test_data.owner.address(),
        manager: test_data.manager.address(),
        enforcement_enabled: true,
    };

    let dex_config = DexConfig {};

    let genesis =
        GenesisConfig::from_minimal_config(genesis_config.into(), blacklist_config, dex_config);

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (test_data, runner)
}

//
// TEST 1 – basic blacklist lifecycle
//
// - DEX enforces not blacklisted for wallet (should succeed: not in blacklist)
// - Manager designates a blacklist signer
// - Signer blacklists wallet
// - DEX enforces not blacklisted (should fail: wallet is blacklisted)
// - Signer removes wallet from blacklist
// - DEX enforces not blacklisted (should succeed again)
// - Owner attempts to set blacklist signer (should fail: only manager allowed)
// - Owner attempts to set blacklisted (should fail: only signer allowed)
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

    // DEX enforces not blacklisted (should succeed: wallet is not in blacklist)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should succeed when wallet is not blacklisted"
            );
        }),
    });

    // Manager sets a blacklist signer
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklistSigner should succeed for manager"
            );
        }),
    });

    // Signer blacklists wallet
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklisted {
                wallet: wallet_addr.clone(),
                blacklisted: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklisted should succeed for authorized blacklist signer"
            );
        }),
    });

    // DEX enforces not blacklisted (should fail: wallet is blacklisted)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should fail when wallet is blacklisted"
            );
        }),
    });

    // Signer removes wallet from blacklist
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklisted {
                wallet: wallet_addr.clone(),
                blacklisted: false,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklisted(false) should succeed and remove wallet from blacklist"
            );
        }),
    });

    // DEX enforces not blacklisted (should succeed again)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should succeed after wallet is removed from blacklist"
            );
        }),
    });

    // Owner sets blacklist signer (should fail because only manager is allowed)
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetBlacklistSigner should fail when called by owner (not manager)"
            );
        }),
    });

    // Owner sets blacklisted (should fail because only signer allowed)
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklisted {
                wallet: wallet_addr.clone(),
                blacklisted: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetBlacklisted should fail when called by non-signer"
            );
        }),
    });
}

//
// TEST 2 – batch blacklisting and per-wallet isolation
//
// - Manager sets a blacklist signer
// - Signer blacklists wallet and wallet2 in batch
// - DEX enforces not blacklisted for both (should fail for both)
// - Signer removes wallet from blacklist
// - DEX enforces not blacklisted for wallet (should succeed)
// - DEX enforces not blacklisted for wallet2 (should still fail)
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

    // Manager sets blacklist signer
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklistSigner should succeed for manager"
            );
        }),
    });

    // Signer blacklists wallet and wallet2 in batch
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistedBatch {
                wallets: vec![wallet_addr.clone(), wallet2_addr.clone()],
                blacklisted: vec![true, true],
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklistedBatch should succeed for authorized signer"
            );
        }),
    });

    // DEX enforces not blacklisted for both wallets (should fail for both)
    for target_wallet in [wallet_addr.clone(), wallet2_addr.clone()] {
        runner.execute_transaction(TransactionTestCase {
            input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
                DexCallMessage::EnforceNotBlacklisted {
                    wallet: target_wallet,
                },
            ),
            assert: Box::new(|result, _state| {
                assert!(
                    !result.tx_receipt.is_successful(),
                    "EnforceNotBlacklisted should fail for all blacklisted wallets"
                );
            }),
        });
    }

    // Signer removes wallet from blacklist
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklisted {
                wallet: wallet_addr.clone(),
                blacklisted: false,
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklisted(false) should remove wallet from blacklist"
            );
        }),
    });

    // DEX enforces not blacklisted for wallet (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should succeed for wallet after removal from blacklist"
            );
        }),
    });

    // DEX enforces not blacklisted for wallet2 (should still fail)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet2_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should still fail for wallet2"
            );
        }),
    });
}

//
// TEST 3 – global enforcement flag and manager change
//
// - Manager sets a blacklist signer
// - Signer blacklists wallet
// - DEX enforces not blacklisted (should fail)
// - Owner disables global enforcement
// - DEX enforces not blacklisted (should succeed: enforcement disabled)
// - Owner changes manager to owner address
// - New manager (owner) can now set blacklist signer successfully
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

    // Manager sets blacklist signer
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklistSigner should succeed for manager"
            );
        }),
    });

    // Signer blacklists wallet
    runner.execute_transaction(TransactionTestCase {
        input: signer.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklisted {
                wallet: wallet_addr.clone(),
                blacklisted: true,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklisted should succeed for authorized signer"
            );
        }),
    });

    // DEX enforces not blacklisted (should fail: wallet is blacklisted)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                !result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should fail when wallet is blacklisted"
            );
        }),
    });

    // Owner disables global enforcement
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetEnforcementEnabled { enabled: false },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetEnforcementEnabled(false) should succeed for owner"
            );
        }),
    });

    // With enforcement disabled, DEX checks should succeed even for blacklisted wallet
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, TestDex<S>>(
            DexCallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "EnforceNotBlacklisted should succeed when enforcement is disabled"
            );
        }),
    });

    // Direct call endpoint should behave the same: no-op under disabled enforcement
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::EnforceNotBlacklisted {
                wallet: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Direct EnforceNotBlacklisted should succeed when enforcement is disabled"
            );
        }),
    });

    // Owner changes manager to owner address
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
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

    // New manager (owner) can now set blacklist signer successfully
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetBlacklistSigner {
                signer: signer_addr.clone(),
                allowed: true,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetBlacklistSigner should succeed once owner has become manager"
            );
        }),
    });
}

//
// TEST 4 – ownership transfer
//
// - Owner transfers ownership to wallet (should succeed)
// - Old owner tries to change manager (should fail: no longer owner)
// - New owner (wallet) changes manager (should succeed)
// - Non-owner (manager) tries to transfer ownership (should fail)
//
#[test]
fn test_transfer_ownership() {
    let (test_data, mut runner) = setup();

    let owner = &test_data.owner;
    let manager = &test_data.manager;
    let wallet = &test_data.wallet;

    let wallet_addr = wallet.address().clone();
    let manager_addr = manager.address().clone();

    // Owner transfers ownership to wallet
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::TransferOwnership {
                new_owner: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "TransferOwnership should succeed for current owner"
            );
        }),
    });

    // Old owner tries to change manager (should fail: no longer owner)
    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetManager {
                new_manager: manager_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                !result.tx_receipt.is_successful(),
                "SetManager should fail for old owner after ownership transfer"
            );
        }),
    });

    // New owner (wallet) changes manager (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: wallet.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::SetManager {
                new_manager: wallet_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "SetManager should succeed for new owner"
            );
        }),
    });

    // Non-owner (manager) tries to transfer ownership (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: manager.create_plain_message::<TestRuntime<S>, Blacklist<S>>(
            CallMessage::TransferOwnership {
                new_owner: manager_addr.clone(),
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                !result.tx_receipt.is_successful(),
                "TransferOwnership should fail for non-owner"
            );
        }),
    });
}
