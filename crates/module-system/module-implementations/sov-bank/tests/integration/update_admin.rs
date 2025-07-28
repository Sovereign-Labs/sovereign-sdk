use sov_bank::utils::TokenHolder;
use sov_bank::{get_token_id, Amount, Bank, TokenId};
use sov_modules_api::{Address, ApiStateAccessor, Error, TxEffect};
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

use crate::helpers::*;

type S = sov_test_utils::TestSpec;
const INITIAL_TOKEN_BALANCE: Amount = Amount::new(1000);

#[derive(Debug, Clone)]
struct Admins {
    current_admin: TestUser<S>,
    new_admin: TestUser<S>,
    admin_1: TestUser<S>,
    admin_2: TestUser<S>,
}

impl Admins {
    fn original_admins(&self) -> Vec<TokenHolder<S>> {
        vec![
            TokenHolder::User(self.admin_1.address()),
            TokenHolder::User(self.current_admin.address()),
            TokenHolder::User(self.admin_2.address()),
        ]
    }

    fn updated_admins(&self) -> Vec<TokenHolder<S>> {
        vec![
            TokenHolder::User(self.admin_1.address()),
            TokenHolder::User(self.new_admin.address()),
            TokenHolder::User(self.admin_2.address()),
        ]
    }
}

fn get_admins(token_id: &TokenId, state: &mut ApiStateAccessor<S>) -> Vec<TokenHolder<S>> {
    let bank = Bank::<S>::default();
    let token = bank.get_token(token_id, state).unwrap().unwrap();
    token.admins().to_vec()
}

fn create_token(
    admins: Admins,
    token_name: &str,
    minter_address: Address,
) -> sov_bank::CallMessage<S> {
    sov_bank::CallMessage::CreateToken {
        token_name: token_name.try_into().unwrap(),
        token_decimals: None,
        initial_balance: INITIAL_TOKEN_BALANCE,
        mint_to_address: minter_address,
        supply_cap: Some(INITIAL_TOKEN_BALANCE),
        admins: vec![
            admins.admin_1.address(),
            admins.current_admin.address(),
            admins.admin_2.address(),
        ]
        .try_into()
        .expect("Tokens can have at least one minter"),
    }
}

#[test]
fn test_update_admin() {
    let (
        TestData {
            minter,
            user_high_token_balance: current_admin,
            another_user_high_token_balance: new_admin,
            ..
        },
        mut runner,
    ) = setup();

    let minter_address = minter.as_user().address();
    let token_name = "Token1";
    let token_id = get_token_id::<S>(token_name, None, &minter_address);

    let admins = Admins {
        current_admin,
        new_admin,
        admin_1: TestUser::<S>::generate(Amount::new(200)),
        admin_2: TestUser::<S>::generate(Amount::new(200)),
    };

    // 1. Create token with some admins.
    {
        let admins = admins.clone();
        runner.execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(create_token(
                admins.clone(),
                token_name,
                minter_address,
            )),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());
                assert_eq!(get_admins(&token_id, state), admins.original_admins());
            }),
        });
    }

    // 2. Fail: Update with existing admin.
    {
        let admins = admins.clone();
        runner.execute_transaction(TransactionTestCase {
            input: admins.current_admin.create_plain_message::<RT, Bank<S>>(
                sov_bank::CallMessage::UpdateAdmin {
                    new_admin: Some(admins.current_admin.address()),
                    token_id,
                },
            ),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_reverted());
                if let TxEffect::Reverted(contents) = result.tx_receipt {
                    let Error::ModuleError(err) = contents.reason;
                    let msg = format!(
                        "`{}` is already a member of the admin list",
                        admins.current_admin.address()
                    );
                    assert!(err.to_string().contains(&msg));
                }

                assert_eq!(get_admins(&token_id, state), admins.original_admins());
            }),
        });
    }

    // 3. Fail: Sender missing from the admin list.
    {
        let admins = admins.clone();
        runner.execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::UpdateAdmin {
                new_admin: Some(admins.new_admin.address()),
                token_id,
            }),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_reverted());
                if let TxEffect::Reverted(contents) = result.tx_receipt {
                    let Error::ModuleError(err) = contents.reason;
                    let msg = format!(
                        "Cannot update admin: `{}` is not in the admin list for the specified token Token1",
                        minter.address()
                    );
                    assert!(err.to_string().contains(&msg));
                }
                assert_eq!(get_admins(&token_id, state), admins.original_admins());
            }),
        });
    }

    // 4. Successful update.
    {
        let admins = admins.clone();
        runner.execute_transaction(TransactionTestCase {
            input: admins.current_admin.create_plain_message::<RT, Bank<S>>(
                sov_bank::CallMessage::UpdateAdmin {
                    new_admin: Some(admins.new_admin.address()),
                    token_id,
                },
            ),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());
                assert_eq!(get_admins(&token_id, state), admins.updated_admins());
            }),
        });
    }

    // 5. Fail: Sender missing from the admin list.
    {
        let admins = admins.clone();
        runner.execute_transaction(TransactionTestCase {
            input: admins.current_admin.create_plain_message::<RT, Bank<S>>(
                sov_bank::CallMessage::UpdateAdmin {
                    new_admin: Some(Address::new([92;28])),
                    token_id,
                },
            ),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_reverted());
                if let TxEffect::Reverted(contents) = result.tx_receipt {
                    let Error::ModuleError(err) = contents.reason;
                    let msg = format!(
                        "Cannot update admin: `{}` is not in the admin list for the specified token Token1",
                        admins.current_admin.address()
                    );
                    assert!(err.to_string().contains(&msg));
                }
                assert_eq!(get_admins(&token_id, state), admins.updated_admins());
            }),
        });
    }

    // 4. Successful removal.
    {
        runner.execute_transaction(TransactionTestCase {
            input: admins.new_admin.create_plain_message::<RT, Bank<S>>(
                sov_bank::CallMessage::UpdateAdmin {
                    new_admin: None,
                    token_id,
                },
            ),
            assert: Box::new(move |result, state| {
                assert!(result.tx_receipt.is_successful());
                assert_eq!(
                    get_admins(&token_id, state),
                    vec![
                        TokenHolder::User(admins.admin_1.address()),
                        TokenHolder::User(admins.admin_2.address()),
                    ]
                );
            }),
        });
    }
}
