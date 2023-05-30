use anyhow::Context;
use borsh::BorshSerialize;
use clap::Parser;
use demo_stf::{runtime::Runtime, sign_tx};
use sov_default_stf::RawTx;
use sov_modules_api::hooks::Transaction;
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey, PublicKey,
    Spec,
};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

type C = DefaultContext;
type Address = <C as Spec>::Address;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct PrivKeyAndAddress {
    hex_priv_key: String,
    address: Address,
}

impl PrivKeyAndAddress {
    fn generate() -> Self {
        let priv_key = DefaultPrivateKey::generate();
        let address = priv_key.pub_key().to_address();
        Self {
            hex_priv_key: priv_key.as_hex(),
            address,
        }
    }

    fn generate_and_save_to_file(priv_key_path: &Path) -> anyhow::Result<()> {
        let priv_key = Self::generate();
        let data = serde_json::to_string(&priv_key)?;
        fs::create_dir_all(priv_key_path)?;
        let path = Path::new(priv_key_path).join(format!("{}.json", priv_key.address));
        fs::write(path, data)?;
        Ok(())
    }
}

#[derive(Parser)]
enum Cli {
    /// Creates a new private key.
    CreatePrivateKey {
        /// Location of the private key.
        priv_key_path: String,
    },
    /// Serializes call message.
    SerializeCall {
        /// Private key used to sign the transaction.
        sender_priv_key_path: String,
        /// Location of the `call message`.
        call_data_path: String,
        /// The `call message` nonce.
        nonce: u64,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli {
        Cli::CreatePrivateKey { priv_key_path } => {
            PrivKeyAndAddress::generate_and_save_to_file(priv_key_path.as_ref())
                .unwrap_or_else(|e| panic!("Create private key error: {}", e));
        }
        Cli::SerializeCall {
            sender_priv_key_path,
            call_data_path,
            nonce,
        } => {
            let serialized = SerializedTx::new(&sender_priv_key_path, &call_data_path, nonce)
                .unwrap_or_else(|e| panic!("Call message serialization error: {}", e));

            let mut bin_path = PathBuf::from(call_data_path);
            bin_path.set_extension("dat");

            let mut file = File::create(bin_path)
                .unwrap_or_else(|e| panic!("Unable to crate .dat file: {}", e));

            file.write_all(&vec![serialized.raw.data].try_to_vec().unwrap())
                .unwrap_or_else(|e| panic!("Unable to save .dat file: {}", e));
        }
    };
}

struct SerializedTx {
    raw: RawTx,
    #[allow(dead_code)]
    sender: Address,
}

impl SerializedTx {
    fn new<P: AsRef<Path>>(
        sender_priv_key_path: P,
        call_data_path: P,
        nonce: u64,
    ) -> anyhow::Result<SerializedTx> {
        let sender_priv_key = Self::deserialize_priv_key(sender_priv_key_path)?;
        let sender_address = sender_priv_key.pub_key().to_address();
        let message = Self::serialize_call_message(call_data_path, &sender_address)?;

        let sig = sign_tx(&sender_priv_key, &message, nonce);
        let tx = Transaction::<C>::new(message, sender_priv_key.pub_key(), sig, nonce);

        Ok(SerializedTx {
            raw: RawTx {
                data: tx.try_to_vec()?,
            },
            sender: sender_address,
        })
    }

    fn deserialize_priv_key<P: AsRef<Path>>(
        sender_priv_key_path: P,
    ) -> anyhow::Result<DefaultPrivateKey> {
        let priv_key_data = std::fs::read_to_string(&sender_priv_key_path).with_context(|| {
            format!(
                "Failed to read private key from {:?}",
                sender_priv_key_path.as_ref()
            )
        })?;

        let sender_priv_key_data = serde_json::from_str::<PrivKeyAndAddress>(&priv_key_data)?;

        Ok(DefaultPrivateKey::from_hex(
            &sender_priv_key_data.hex_priv_key,
        )?)
    }

    fn serialize_call_message<P: AsRef<Path>>(
        call_data_path: P,
        sender_address: &Address,
    ) -> anyhow::Result<Vec<u8>> {
        let call_data = std::fs::read_to_string(&call_data_path).with_context(|| {
            format!(
                "Failed to read call data from {:?}",
                call_data_path.as_ref()
            )
        })?;

        let call_msg = serde_json::from_str::<sov_bank::call::CallMessage<C>>(&call_data)?;

        if let sov_bank::call::CallMessage::CreateToken {
            salt, token_name, ..
        } = &call_msg
        {
            let token_address =
                sov_bank::create_token_address::<C>(token_name, sender_address.as_ref(), *salt);

            println!(
                "This message will crate a new Token with Address: {}",
                token_address
            );
        }

        Ok(Runtime::<C>::encode_bank_call(call_msg))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use demo_stf::app::{DemoApp, DemoAppRunner};
    use demo_stf::genesis_config::{
        create_demo_genesis_config, generate_address, DEMO_SEQUENCER_DA_ADDRESS,
        DEMO_SEQ_PUB_KEY_STR, LOCKED_AMOUNT,
    };
    use demo_stf::runner_config::Config;
    use demo_stf::runtime::GenesisConfig;
    use sov_default_stf::{Batch, RawTx, SequencerOutcome};
    use sov_modules_api::Address;
    use sov_rollup_interface::stf::StateTransitionRunner;

    use sov_rollup_interface::{mocks::MockZkvm, stf::StateTransitionFunction};
    use sov_state::WorkingSet;

    #[test]
    fn test_cmd() {
        let mut test_demo = TestDemo::new();
        let test_data = read_test_data();

        execute_txs(&mut test_demo.demo, test_demo.config, test_data.data);

        // get minter balance
        let balance = get_balance(
            &mut test_demo.demo,
            &test_data.token_deployer_address,
            test_data.minter_address,
        );

        // The minted amount was 1000 and we transferred 200 and burned 300.
        assert_eq!(balance, Some(500))
    }

    // Test helpers
    struct TestDemo {
        config: demo_stf::runtime::GenesisConfig<C>,
        demo: DemoApp<C, MockZkvm>,
    }

    impl TestDemo {
        fn new() -> Self {
            let path = sov_schema_db::temppath::TempPath::new();
            let value_setter_admin_private_key = DefaultPrivateKey::generate();
            let election_admin_private_key = DefaultPrivateKey::generate();

            let genesis_config = create_demo_config(
                LOCKED_AMOUNT + 1,
                &value_setter_admin_private_key,
                &election_admin_private_key,
            );

            let path = path.as_ref().to_path_buf();
            let runner_config = Config {
                storage: sov_state::config::Config { path },
            };

            Self {
                config: genesis_config,
                demo: DemoAppRunner::<DefaultContext, MockZkvm>::new(runner_config).0,
            }
        }
    }

    struct TestData {
        token_deployer_address: Address,
        minter_address: Address,
        data: Vec<RawTx>,
    }

    fn make_test_path<P: AsRef<Path>>(path: P) -> PathBuf {
        let mut sender_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        sender_path.push("src");
        sender_path.push("bank_cmd");
        sender_path.push("test_data");

        sender_path.push(path);

        sender_path
    }

    fn read_test_data() -> TestData {
        let create_token = SerializedTx::new(
            make_test_path("token_deployer_private_key.json"),
            make_test_path("create_token.json"),
            0,
        )
        .unwrap();

        let transfer = SerializedTx::new(
            make_test_path("minter_private_key.json"),
            make_test_path("transfer.json"),
            0,
        )
        .unwrap();

        let burn = SerializedTx::new(
            make_test_path("minter_private_key.json"),
            make_test_path("burn.json"),
            1,
        )
        .unwrap();

        let data = vec![create_token.raw, transfer.raw, burn.raw];

        TestData {
            token_deployer_address: create_token.sender,
            minter_address: transfer.sender,
            data,
        }
    }

    fn execute_txs(
        demo: &mut DemoApp<C, MockZkvm>,
        config: demo_stf::runtime::GenesisConfig<C>,
        txs: Vec<RawTx>,
    ) {
        StateTransitionFunction::<MockZkvm>::init_chain(demo, config);
        StateTransitionFunction::<MockZkvm>::begin_slot(demo, Default::default());

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm>::apply_blob(
            demo,
            new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        )
        .inner;
        assert!(
            matches!(apply_blob_outcome, SequencerOutcome::Rewarded,),
            "Sequencer execution should have succeeded but failed "
        );
        StateTransitionFunction::<MockZkvm>::end_slot(demo);
    }

    fn get_balance(
        demo: &mut DemoApp<DefaultContext, MockZkvm>,
        token_deployer_address: &Address,
        user_address: Address,
    ) -> Option<u64> {
        let token_address = create_token_address(token_deployer_address);

        let mut working_set = WorkingSet::new(demo.current_storage.clone());

        let balance = demo
            .runtime
            .bank
            .balance_of(user_address, token_address, &mut working_set);

        balance.amount
    }

    fn create_token_address(token_deployer_address: &Address) -> Address {
        sov_bank::create_token_address::<C>("sov-test-token", token_deployer_address.as_ref(), 11)
    }

    pub type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

    pub fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
        let address = Address::try_from(address).unwrap();
        let data = batch.try_to_vec().unwrap();
        TestBlob::new(data, address)
    }

    pub fn create_demo_config(
        initial_sequencer_balance: u64,
        value_setter_admin_private_key: &DefaultPrivateKey,
        election_admin_private_key: &DefaultPrivateKey,
    ) -> GenesisConfig<DefaultContext> {
        create_demo_genesis_config::<DefaultContext>(
            initial_sequencer_balance,
            generate_address::<DefaultContext>(DEMO_SEQ_PUB_KEY_STR),
            DEMO_SEQUENCER_DA_ADDRESS.to_vec(),
            value_setter_admin_private_key,
            election_admin_private_key,
        )
    }
}
