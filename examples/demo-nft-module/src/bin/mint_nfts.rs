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

fn run_sov_cli_import(json_str: &str) {
    let output = Command::new("cargo")
        .args(&["run", "--bin", "sov-cli", "transactions", "import", "from-string", "nft", "--json", json_str])
        .current_dir(Path::new("../.."))
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    println!("stdout: {}", stdout);
    println!("stderr: {}", stderr);
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

fn mint_collection(collection_name: &str, sender_address: &[u8]) {
    let c_addr = get_collection_address::<DefaultContext>(COLLECTION_1,signer_address);
    let murl = get_collection_metadata_url(&c_addr.to_string());
    let c = CallMessage::<DefaultContext>::CreateCollection
    {
        name: collection_name.to_string(),
        metadata_url: murl.to_string(),
    };
}
fn mint_nfts(collection_address: &str, nft_id: u64) {
}
fn execute_transfer() {

}

fn main() {
    let (signer_address_str, signer_pub_key_str)= get_signing_address_pubkey();
    let signer_pub_key = DefaultPublicKey::from_str(&signer_pub_key_str).unwrap();
    let binding = signer_pub_key.to_address::<<DefaultContext as Spec>::Address>();
    let signer_address = binding.as_ref();
    // let ca = get_collection_address::<DefaultContext>(COLLECTION_1,signer_address);
    println!("{:?}",ca);
    mint_collection(COLLECTION_1,signer_address);
    mint_collection(COLLECTION_2,signer_address);
    mint_collection(COLLECTION_3,signer_address);
}