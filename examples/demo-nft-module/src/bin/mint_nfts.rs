use std::path::Path;
use jsonrpsee::core::__reexports::serde_json;
use demo_nft_module::CallMessage;
use demo_nft_module::utils::get_collection_address;
use sov_modules_api::default_context::{DefaultContext};
use sov_modules_api::default_signature::DefaultPublicKey;
use std::process::Command;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use sov_modules_api::{PublicKey, Spec};

const COLLECTION_1: &str = "collection_1";
const COLLECTION_2: &str = "collection_2";
const COLLECTION_3: &str = "collection_3";

const DUMMY_URL: &str = "http://foobar";

fn get_collection_metadata_url(collection_address: &str) -> String {
    format!("{}/collection/{}",DUMMY_URL,collection_address)
}

fn get_nft_metadata_url(collection_address: &str, nft_id: u64) -> String {
    format!("{}/nft/{}/{}",DUMMY_URL,collection_address,nft_id)
}

fn run_sov_cli(args: &[&str]) {
    let mut complete_args = vec!["run", "--bin", "sov-cli"];
    complete_args.extend_from_slice(args);

    let output = Command::new("cargo")
        .args(&complete_args)
        .current_dir(Path::new("../.."))
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    println!("stdout: {}", stdout);
    println!("stderr: {}", stderr);
}

fn run_sov_cli_import(json_str: &str) {
    let specific_args = ["transactions", "import", "from-string", "nft", "--json", json_str];
    run_sov_cli(&specific_args);
}

fn run_sov_cli_submit_batch() {
    let specific_args = ["rpc", "submit-batch"];
    run_sov_cli(&specific_args);
}

fn run_sov_cli_clean_batch() {
    let specific_args = ["transactions", "clean"];
    run_sov_cli(&specific_args);
}

fn run_sov_cli_generate_keys_if_missing() {
    let specific_args = ["keys", "generate-if-missing"];
    run_sov_cli(&specific_args);
}

#[derive(Debug, Deserialize)]
struct Address {
    address: String,
    nickname: Option<String>,
    location: String,
    pub_key: String,
}

#[derive(Debug, Deserialize)]
struct AddressesOutput {
    addresses: Vec<Address>,
}

pub fn parse_address_and_pub_key_from_json(json_str: &str) -> Option<(String, String)> {
    let parsed: Result<serde_json::Value, serde_json::Error> = serde_json::from_str(json_str);

    match parsed {
        Ok(json_value) => {
            let addresses = json_value["addresses"].as_array()?;

            let first_address = addresses.get(0)?;
            let address = first_address["address"].as_str()?;
            let mut pub_key = first_address["pub_key"].as_str()?;

            // Remove the "0x" prefix if it exists
            if pub_key.starts_with("0x") || pub_key.starts_with("0X") {
                pub_key = &pub_key[2..];
            }

            Some((address.to_string(), pub_key.to_string()))
        }
        Err(_) => None,
    }
}

fn get_signing_address_pubkey() -> (String,String) {
    let output = Command::new("cargo")
        .args(&["run", "--bin", "sov-cli", "keys", "list"])
        .current_dir(Path::new("../.."))
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _stderr = String::from_utf8(output.stderr).unwrap();
    parse_address_and_pub_key_from_json(&stdout).unwrap()
}

fn mint_collection(collection_name: &str, signer_address: &[u8]) {
    let c_addr = get_collection_address::<DefaultContext>(collection_name,signer_address);
    let murl = get_collection_metadata_url(&c_addr.to_string());
    let c = CallMessage::<DefaultContext>::CreateCollection
    {
        name: collection_name.to_string(),
        metadata_url: murl.to_string(),
    };
    let json_str = serde_json::to_string(&c).unwrap();
    run_sov_cli_import(&json_str);
}

fn mint_nft_to_collection(nft_id: u64, collection_name: &str, signer_address: &[u8], mint_to: <DefaultContext as Spec>::Address) {
    let c_addr = get_collection_address::<DefaultContext>(COLLECTION_1,signer_address);
    let nurl = get_nft_metadata_url(&c_addr.to_string(), nft_id);
    let m = CallMessage::<DefaultContext>::MintNft {
        collection_name: collection_name.to_string(),
        metadata_url: nurl,
        id: nft_id,
        mint_to_address: mint_to,
        frozen: false,
    };
    let json_str = serde_json::to_string(&m).unwrap();
    run_sov_cli_import(&json_str);
}

fn execute_transfer() {

}

fn main() {
    let (_, signer_pub_key_str)= get_signing_address_pubkey();
    let signer_pub_key = DefaultPublicKey::from_str(&signer_pub_key_str).unwrap();
    let binding = signer_pub_key.to_address::<<DefaultContext as Spec>::Address>();
    let signer_address = binding.as_ref();

    run_sov_cli_generate_keys_if_missing();
    run_sov_cli_clean_batch();
    mint_collection(COLLECTION_1,signer_address);
    mint_collection(COLLECTION_2,signer_address);
    mint_collection(COLLECTION_3,signer_address);
    run_sov_cli_submit_batch();
}