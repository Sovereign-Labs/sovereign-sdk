use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{fs, vec};

use anyhow::Context;
use borsh::BorshSerialize;
use clap::Parser;
use const_rollup_config::ROLLUP_NAMESPACE_RAW;
use demo_stf::runtime::cmd_parser;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::HttpClientBuilder;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{AddressBech32, PublicKey, Spec};
pub use sov_modules_stf_template::RawTx;
use sov_sequencer::SubmitTransaction;

type C = DefaultContext;
type Address = <C as Spec>::Address;

/// Main entry point for CLI
#[derive(Parser)]
#[clap(version = "1.0", author = "Sovereign")]
struct Cli {
    #[clap(subcommand)]
    /// Commands to perform operations
    command: Commands,
}

/// Main commands
#[derive(Parser)]
enum Commands {
    /// Serialize a call to a module.
    /// This creates a dat file containing the serialized transaction
    SerializeCall {
        /// Path to the json file containing the private key of the sender
        sender_priv_key_path: String,
        /// Name of the module to generate the call.
        /// Modules defined in your Runtime are supported.
        /// (eg: Bank, Accounts)
        module_name: String,
        /// Path to the json file containing the parameters for a module call
        call_data_path: String,
        /// Nonce for the transaction
        nonce: u64,
    },
    /// Submits transaction to sequencer
    SubmitCall {
        /// Path to the json file containing the private key of the sender
        sender_priv_key_path: String,
        /// Name of the module to generate the call.
        /// Modules defined in your Runtime are supported.
        /// (eg: Bank, Accounts)
        module_name: String,
        /// Path to the json file containing the parameters for a module call
        call_data_path: String,
        /// Nonce for the transaction
        nonce: u64,
        /// RPC endpoint with sequencer RPC
        rpc_endpoint: String,
    },
    /// Tells Sequencer to publish batch
    PublishBatch {
        /// RPC endpoint with sequencer RPC
        rpc_endpoint: String,
    },
    /// Combine a list of files generated by SerializeCall into a blob for submission to Celestia
    MakeBlob {
        /// List of files containing serialized transactions
        path_list: Vec<String>,
    },
    /// Utility commands
    Util(UtilArgs),
}

/// Arguments for utility commands
#[derive(Parser)]
struct UtilArgs {
    #[clap(subcommand)]
    /// Commands under utilities
    command: UtilCommands,
}

/// List of utility commands
#[derive(Parser)]
enum UtilCommands {
    /// Compute the address of a derived token. This follows a deterministic algorithm
    DeriveTokenAddress {
        /// Name of the token
        token_name: String,
        /// Address of the sender (can be obtained using the show-public-key subcommand)
        sender_address: String,
        /// A unique random number
        salt: u64,
    },
    /// Display the public key associated with a private key
    ShowPublicKey {
        /// Path to the json file containing the private key
        private_key_path: String,
    },
    /// Create a new private key
    CreatePrivateKey {
        /// Folder to store the new private key json file. The filename is auto-generated
        priv_key_path: String,
    },
    PrintNamespace,
}

struct SerializedTx {
    raw: RawTx,
    #[allow(dead_code)]
    sender: Address,
}

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
        fs::write(&path, data)?;
        println!(
            "private key written to path: {}",
            path.into_os_string().into_string().unwrap()
        );
        Ok(())
    }
}

impl SerializedTx {
    fn new<P: AsRef<Path>>(
        sender_priv_key_path: P,
        module_name: &str,
        call_data_path: P,
        nonce: u64,
    ) -> anyhow::Result<SerializedTx> {
        let sender_priv_key = Self::deserialize_priv_key(sender_priv_key_path)?;
        let sender_address = sender_priv_key.pub_key().to_address();
        let message = Self::serialize_call_message(module_name, call_data_path)?;

        let tx = Transaction::<C>::new_signed_tx(&sender_priv_key, message, nonce);

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
        module_name: &str,
        call_data_path: P,
    ) -> anyhow::Result<Vec<u8>> {
        let call_data = std::fs::read_to_string(&call_data_path).with_context(|| {
            format!(
                "Failed to read call data from {:?}",
                call_data_path.as_ref()
            )
        })?;
        cmd_parser(module_name, &call_data)
    }
}

fn serialize_call(command: &Commands) -> String {
    if let Commands::SerializeCall {
        sender_priv_key_path,
        module_name,
        call_data_path,
        nonce,
    } = command
    {
        let serialized =
            SerializedTx::new(&sender_priv_key_path, module_name, &call_data_path, *nonce)
                .unwrap_or_else(|e| panic!("Call message serialization error: {}", e));

        hex::encode(serialized.raw.data)
    } else {
        Default::default()
    }
}

fn make_hex_blob(txs: impl Iterator<Item = String>) -> String {
    // decode the hex string to bytes
    let mut batch = vec![];
    for tx in txs {
        let bytes = hex::decode(tx.as_bytes()).expect("Decoding failed");
        batch.push(bytes);
    }
    hex::encode(batch.try_to_vec().unwrap())
}

#[tokio::main]
pub async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::SerializeCall {
            ref call_data_path, ..
        } => {
            let raw_contents = serialize_call(&cli.command);
            let mut bin_path = PathBuf::from(call_data_path);
            bin_path.set_extension("dat");

            let mut file = File::create(bin_path)
                .unwrap_or_else(|e| panic!("Unable to crate .dat file: {}", e));
            file.write_all(raw_contents.as_bytes())
                .unwrap_or_else(|e| panic!("Unable to save .dat file: {}", e));
        }
        Commands::SubmitCall {
            sender_priv_key_path,
            module_name,
            call_data_path,
            nonce,
            rpc_endpoint,
        } => {
            let serialized =
                SerializedTx::new(&sender_priv_key_path, &module_name, &call_data_path, nonce)
                    .unwrap_or_else(|e| panic!("Call message serialization error: {}", e));

            let request = SubmitTransaction::new(serialized.raw.data);
            let client = HttpClientBuilder::default().build(rpc_endpoint).unwrap();
            let response: Result<String, _> = client.request("sequencer_acceptTx", [request]).await;

            println!("Transaction submit result: {:?}", response);
        }
        Commands::PublishBatch { rpc_endpoint } => {
            let client = HttpClientBuilder::default().build(rpc_endpoint).unwrap();

            let response: Result<(), _> = client.request("sequencer_publishBatch", [1u32]).await;

            // Print the result
            println!("Publish batch result: {:?}", response);
        }
        Commands::MakeBlob { path_list } => {
            let mut hex_encoded_txs = vec![];
            for path in path_list {
                let mut f = File::open(path).expect("Unable to open file");
                let mut hex_string = String::new();
                f.read_to_string(&mut hex_string)
                    .expect("Unable to read the file");
                // push it into data_list
                hex_encoded_txs.push(hex_string);
            }

            let blob = make_hex_blob(hex_encoded_txs.into_iter());
            println!("{}", blob)
        }
        Commands::Util(util_args) => match util_args.command {
            UtilCommands::DeriveTokenAddress {
                token_name,
                sender_address,
                salt,
            } => {
                let sender_address = Address::from(
                    AddressBech32::try_from(sender_address.clone()).unwrap_or_else(|e| {
                        panic!(
                            "Failed to derive pub key from string: {}: {}",
                            sender_address, e
                        )
                    }),
                );
                let token_address =
                    sov_bank::get_token_address::<C>(&token_name, sender_address.as_ref(), salt);
                println!("{}", token_address);
            }

            UtilCommands::ShowPublicKey { private_key_path } => {
                let sender_priv_key = SerializedTx::deserialize_priv_key(private_key_path)
                    .expect("Failed to get private key from file");
                let sender_address: Address = sender_priv_key.pub_key().to_address();
                println!("{}", sender_address);
            }

            UtilCommands::CreatePrivateKey { priv_key_path } => {
                PrivKeyAndAddress::generate_and_save_to_file(priv_key_path.as_ref())
                    .unwrap_or_else(|e| panic!("Create private key error: {}", e));
            }
            UtilCommands::PrintNamespace => {
                println!("{}", hex::encode(ROLLUP_NAMESPACE_RAW));
            }
        },
    }
}

#[cfg(test)]
mod test {
    use borsh::BorshDeserialize;
    use demo_stf::app::{DemoApp, DemoAppRunner};
    use demo_stf::genesis_config::{create_demo_config, DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT};
    use demo_stf::runner_config::Config;
    use demo_stf::runtime::GenesisConfig;
    use sov_modules_api::Address;
    use sov_modules_stf_template::{Batch, RawTx, SequencerOutcome};
    use sov_rollup_interface::mocks::MockZkvm;
    use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
    use sov_rollup_interface::stf::StateTransitionFunction;
    use sov_state::WorkingSet;

    use super::*;

    type TestBlob = sov_rollup_interface::mocks::TestBlob<Address>;

    fn new_test_blob(batch: Batch, address: &[u8]) -> TestBlob {
        let address = Address::try_from(address).unwrap();
        let data = batch.try_to_vec().unwrap();
        TestBlob::new(data, address, [0; 32])
    }

    #[test]
    fn test_sov_cli() {
        // Tempdir is created here, so it will be deleted only after test is finished.
        let tempdir = tempfile::tempdir().unwrap();
        let mut test_demo = TestDemo::with_path(tempdir.path().to_path_buf());
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

    #[test]
    fn test_create_token() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut test_demo = TestDemo::with_path(tempdir.path().to_path_buf());
        let test_tx = serialize_call(&Commands::SerializeCall {
            sender_priv_key_path: make_test_path("keys/token_deployer_private_key.json")
                .to_str()
                .unwrap()
                .into(),
            module_name: "Bank".into(),
            call_data_path: make_test_path("requests/create_token.json")
                .to_str()
                .unwrap()
                .into(),
            nonce: 0,
        });

        let mut test_data = read_test_data();
        test_data.data.pop();
        test_data.data.pop();

        let batch = Batch {
            txs: test_data.data.clone(),
        };

        println!("batch: {}", hex::encode(batch.try_to_vec().unwrap()));

        let blob = make_hex_blob(vec![test_tx].into_iter());
        println!("generated: {}", &blob);

        // let mut blob = hex::decode(blob.as_bytes()).expect("hex is valid"):
        let blob = hex::decode(blob.as_bytes()).unwrap();

        let batch = Batch::deserialize(&mut &blob[..]).expect("must be valid blob");
        execute_txs(&mut test_demo.demo, test_demo.config, batch.txs);
    }

    // Test helpers
    struct TestDemo {
        config: GenesisConfig<C>,
        demo: DemoApp<C, MockZkvm, TestBlob>,
    }

    impl TestDemo {
        fn with_path(path: PathBuf) -> Self {
            let value_setter_admin_private_key = DefaultPrivateKey::generate();
            let election_admin_private_key = DefaultPrivateKey::generate();

            let genesis_config = create_demo_config(
                LOCKED_AMOUNT + 1,
                &value_setter_admin_private_key,
                &election_admin_private_key,
            );

            let runner_config = Config {
                storage: sov_state::config::Config { path },
            };

            Self {
                config: genesis_config,
                demo: DemoAppRunner::<DefaultContext, MockZkvm, TestBlob>::new(runner_config).stf,
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
        sender_path.push("..");
        sender_path.push("test-data");

        sender_path.push(path);

        sender_path
    }

    fn read_test_data() -> TestData {
        let create_token = SerializedTx::new(
            make_test_path("keys/token_deployer_private_key.json"),
            "Bank",
            make_test_path("requests/create_token.json"),
            0,
        )
        .unwrap();

        let transfer = SerializedTx::new(
            make_test_path("keys/minter_private_key.json"),
            "Bank",
            make_test_path("requests/transfer.json"),
            0,
        )
        .unwrap();

        let burn = SerializedTx::new(
            make_test_path("keys/minter_private_key.json"),
            "Bank",
            make_test_path("requests/burn.json"),
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
        demo: &mut DemoApp<C, MockZkvm, TestBlob>,
        config: GenesisConfig<C>,
        txs: Vec<RawTx>,
    ) {
        StateTransitionFunction::<MockZkvm, TestBlob>::init_chain(demo, config);
        StateTransitionFunction::<MockZkvm, TestBlob>::begin_slot(demo, Default::default());

        let apply_blob_outcome = StateTransitionFunction::<MockZkvm, TestBlob>::apply_blob(
            demo,
            &mut new_test_blob(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS),
            None,
        )
        .inner;
        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome,
            "Sequencer execution should have succeeded but failed",
        );
        StateTransitionFunction::<MockZkvm, TestBlob>::end_slot(demo);
    }

    fn get_balance(
        demo: &mut DemoApp<DefaultContext, MockZkvm, TestBlob>,
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
        sov_bank::get_token_address::<C>("sov-test-token", token_deployer_address.as_ref(), 11)
    }
}
