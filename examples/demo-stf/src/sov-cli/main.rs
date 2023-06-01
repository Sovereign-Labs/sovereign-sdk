use anyhow::Context;
use borsh::BorshSerialize;
use clap::{Parser};
use sov_modules_api::transaction::Transaction;
use sov_modules_stf_template::RawTx;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use demo_stf::runtime::cmd_parser;

use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey,
    AddressBech32, PublicKey, Spec,
};

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

        let sig = Transaction::<C>::sign(&sender_priv_key, &message, nonce);
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
        module_name: &str,
        call_data_path: P,
    ) -> anyhow::Result<Vec<u8>> {
        let call_data = std::fs::read_to_string(&call_data_path).with_context(|| {
            format!(
                "Failed to read call data from {:?}",
                call_data_path.as_ref()
            )
        })?;
        cmd_parser(&module_name, &call_data)
    }
}

pub fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::SerializeCall {
            sender_priv_key_path,
            module_name,
            call_data_path,
            nonce,
        } => {
            let serialized =
                SerializedTx::new(&sender_priv_key_path, &module_name, &call_data_path, nonce)
                    .unwrap_or_else(|e| panic!("Call message serialization error: {}", e));

            let mut bin_path = PathBuf::from(call_data_path);
            bin_path.set_extension("dat");

            let mut file = File::create(bin_path)
                .unwrap_or_else(|e| panic!("Unable to crate .dat file: {}", e));

            file.write_all(&vec![serialized.raw.data].try_to_vec().unwrap())
                .unwrap_or_else(|e| panic!("Unable to save .dat file: {}", e));
        }
        Commands::Util(util_args) => match util_args.command {
            UtilCommands::DeriveTokenAddress {
                token_name,
                sender_address,
                salt,
            } => {
                let sender_address =
                    Address::from(AddressBech32::try_from(sender_address.clone()).expect(
                        &format!("Failed to derive pub key from string: {}", sender_address),
                    ));
                let token_address =
                    sov_bank::create_token_address::<C>(&token_name, sender_address.as_ref(), salt);
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
        },
    }
}
