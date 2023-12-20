use helpers::*;
use sov_bank::{
    get_genesis_token_address, Bank, BankConfig, BankGasConfig, CallMessage, TokenConfig,
};
use sov_modules_api::macros::config_constant;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, GasUnit, Module, WorkingSet};
use sov_prover_storage_manager::new_orphan_storage;
use tempfile::TempDir;

mod helpers;

const CREATE_TOKEN_NATIVE_COST: u64 = 2;
const CREATE_TOKEN_ZK_COST: u64 = 3;

#[test]
fn zeroed_price_wont_deduct_working_set() {
    let sender_balance = 100;
    let remaining_funds = BankGasTestCase::init(sender_balance).execute().unwrap();

    assert_eq!(
        remaining_funds, sender_balance,
        "the balance should be unchanged with zeroed price"
    );
}

#[test]
fn normal_price_will_deduct_working_set() {
    let sender_balance = 100;

    let native_price = 2;
    let zk_price = 3;
    let remaining_funds = BankGasTestCase::init(sender_balance)
        .with_native_price(native_price)
        .with_zk_price(zk_price)
        .override_gas_config()
        .execute()
        .unwrap();

    // compute the expected gas cost, based on the test constants
    let gas_used = native_price * CREATE_TOKEN_NATIVE_COST + zk_price * CREATE_TOKEN_ZK_COST;

    assert_eq!(
        remaining_funds,
        sender_balance - gas_used,
        "the sender balance is enough for this call"
    );
}

#[test]
fn constants_price_is_charged_correctly() {
    let sender_balance = 100;

    let native_price = 2;
    let zk_price = 3;
    let remaining_funds = BankGasTestCase::init(sender_balance)
        .with_native_price(native_price)
        .with_zk_price(zk_price)
        .execute()
        .unwrap();

    // compute the expected gas cost, based on the json constants
    let bank = Bank::<C>::default();
    let config = bank.gas_config();
    let gas_price = <C as Context>::GasUnit::from_arbitrary_dimensions(&[native_price, zk_price]);
    let gas_used = config.create_token.value(&gas_price);

    assert_eq!(
        remaining_funds,
        sender_balance - gas_used,
        "the sender balance is enough for this call"
    );
}

#[test]
fn not_enough_gas_wont_panic() {
    let sender_balance = 100;

    let result = BankGasTestCase::init(sender_balance)
        .with_native_price(2000)
        .with_zk_price(3000)
        .override_gas_config()
        .execute();

    assert!(
        result.is_err(),
        "the sender balance is not enough for this call"
    );
}

#[test]
fn very_high_gas_price_wont_panic_or_overflow() {
    let sender_balance = 100;

    let result = BankGasTestCase::init(sender_balance)
        .with_native_price(u64::MAX)
        .with_zk_price(3)
        .override_gas_config()
        .execute();

    assert!(result.is_err(), "arithmetic overflow shoulnd't panic");
}

#[allow(dead_code)]
pub struct BankGasTestCase {
    ws: WorkingSet<C>,
    bank: Bank<C>,
    ctx: C,
    message: CallMessage<C>,
    tmpdir: TempDir,
    gas_limit: u64,
    native_price: u64,
    zk_price: u64,
}

impl BankGasTestCase {
    pub fn init(sender_balance: u64) -> Self {
        #[config_constant]
        const GAS_TOKEN_ADDRESS: &'static str;
        let tmpdir = tempfile::tempdir().unwrap();

        // create a base token with an initial balance to pay for the gas
        let base_token_name = "sov-gas-token";
        let salt = 0;

        // sanity check the token address
        let base_token_address = get_genesis_token_address::<C>(base_token_name, salt);
        assert_eq!(&base_token_address.to_string(), GAS_TOKEN_ADDRESS);

        // generate a token configuration with the provided arguments
        let sender_address = generate_address::<C>("sender");
        let address_and_balances = vec![(sender_address, sender_balance)];
        let authorized_minters = vec![];
        let bank_config: BankConfig<C> = BankConfig {
            tokens: vec![TokenConfig {
                token_name: base_token_name.to_string(),
                address_and_balances,
                authorized_minters,
                salt,
            }],
        };

        // create a context using the generated account as sender
        let height = 1;
        let minter_address = generate_address::<C>("minter");
        let sequencer_address = generate_address::<C>("sequencer");
        let ctx = C::new(sender_address, sequencer_address, height);

        // create a bank instance
        let bank = Bank::default();
        let storage = new_orphan_storage(tmpdir.path()).unwrap();
        let mut ws = WorkingSet::new(storage);
        bank.genesis(&bank_config, &mut ws).unwrap();

        // sanity test the sender balance
        let balance = bank.get_balance_of(sender_address, base_token_address, &mut ws);
        assert_eq!(balance, Some(sender_balance));

        // generate a create dummy token message
        let token_name = "dummy".to_string();
        let initial_balance = 500;
        let message = CallMessage::CreateToken::<C> {
            salt,
            token_name,
            initial_balance,
            minter_address,
            authorized_minters: vec![minter_address],
        };

        Self {
            ws,
            bank,
            ctx,
            message,
            tmpdir,
            gas_limit: sender_balance,
            native_price: 0,
            zk_price: 0,
        }
    }

    pub fn override_gas_config(mut self) -> Self {
        self.bank.override_gas_config(BankGasConfig {
            create_token: [CREATE_TOKEN_NATIVE_COST, CREATE_TOKEN_ZK_COST],
            transfer: Default::default(),
            burn: Default::default(),
            mint: Default::default(),
            freeze: Default::default(),
        });
        self
    }

    pub fn with_native_price(mut self, price: u64) -> Self {
        self.native_price = price;
        self
    }

    pub fn with_zk_price(mut self, price: u64) -> Self {
        self.zk_price = price;
        self
    }

    pub fn execute(self) -> anyhow::Result<u64> {
        let Self {
            mut ws,
            bank,
            ctx,
            message,
            tmpdir,
            gas_limit,
            native_price,
            zk_price,
        } = self;

        ws.set_gas(gas_limit, [native_price, zk_price]);
        bank.call(message, &ctx, &mut ws)?;

        // can unlock storage dir
        let _ = tmpdir;

        Ok(ws.gas_remaining_funds())
    }
}
