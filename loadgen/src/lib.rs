use std::{collections::BTreeMap, fs, sync::Mutex, time::Duration};

use abi_stable::std_types::{ROption::*, RResult, RString};
use anyhow::{anyhow, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::Signer;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use loadr_plugin_api::abi::{
    FfiDataSource, FfiDataSourceBox, FfiDataSource_TO, PluginMod, LOADR_PLUGIN_ABI_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use sov_universal_wallet::schema::{chain_hash_fragment, RollupRoots, Schema};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Crypto {
    Ed25519,
    Secp256k1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    url: String,
    keys_path: String,
    crypto: Crypto,
    runtime_call: Value,
    max_fee: String,
}

#[derive(Deserialize)]
struct Init {
    sources: BTreeMap<String, Config>,
    vus: usize,
    vu_offset: usize,
}

enum Key {
    Ed25519(ed25519_dalek::SigningKey),
    Secp256k1(k256::ecdsa::SigningKey),
}

impl Key {
    fn new(crypto: Crypto, bytes: &[u8]) -> Result<Self> {
        Ok(match crypto {
            Crypto::Ed25519 => Self::Ed25519(ed25519_dalek::SigningKey::from_bytes(
                bytes
                    .try_into()
                    .context("expected a 32-byte Ed25519 seed")?,
            )),
            Crypto::Secp256k1 => Self::Secp256k1(k256::ecdsa::SigningKey::from_slice(bytes)?),
        })
    }

    fn public_key(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.verifying_key().to_bytes().to_vec(),
            Self::Secp256k1(key) => key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .to_vec(),
        }
    }

    fn credential(&self) -> String {
        let mut id = [0; 32];
        match self {
            Self::Ed25519(key) => id = key.verifying_key().to_bytes(),
            Self::Secp256k1(key) => {
                let public = key.verifying_key().to_encoded_point(false);
                id[12..].copy_from_slice(&Keccak256::digest(&public.as_bytes()[1..])[12..]);
            }
        }
        hex::encode(id)
    }

    fn sign(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Ed25519(key) => key.sign(bytes).to_bytes().to_vec(),
            Self::Secp256k1(key) => {
                let signature: k256::ecdsa::Signature =
                    key.sign_prehash(&Keccak256::digest(bytes))?;
                signature.to_bytes().to_vec()
            }
        })
    }
}

struct Account {
    key: Key,
    nonce_url: String,
    nonce: Mutex<Option<u64>>,
}

struct Feeder {
    config: Config,
    schema: Schema,
    chain_hash: [u8; 32],
    accounts: Vec<Account>,
    details: Value,
    legacy: bool,
}

impl Feeder {
    fn new(init: Init, schema: Value) -> Result<Self> {
        ensure!(
            init.sources.len() == 1,
            "use exactly one plugin data source"
        );
        let config = init.sources.into_values().next().unwrap();
        let keys: Vec<String> = serde_json::from_slice(&fs::read(&config.keys_path)?)?;
        let end = init
            .vu_offset
            .checked_add(init.vus)
            .context("VU range overflow")?;
        ensure!(
            end <= keys.len(),
            "need at least {end} funded keys for this VU placement"
        );
        let mut seen = std::collections::BTreeSet::new();
        let mut accounts = Vec::with_capacity(init.vus);
        for (i, secret) in keys.iter().enumerate() {
            let key = Key::new(
                config.crypto,
                &hex::decode(secret.trim_start_matches("0x"))?,
            )
            .with_context(|| format!("invalid key at index {i}"))?;
            let credential = key.credential();
            ensure!(
                seen.insert(credential.clone()),
                "duplicate key at index {i}"
            );
            if (init.vu_offset..end).contains(&i) {
                accounts.push(Account {
                    nonce_url: format!(
                        "{}/rollup/addresses/{credential}/dedup?select=nonce",
                        config.url.trim_end_matches('/')
                    ),
                    key,
                    nonce: Mutex::new(None),
                });
            }
        }
        let chain_hash: [u8; 32] = hex::decode(
            schema["chain_hash"]
                .as_str()
                .context("missing chain_hash")?
                .trim_start_matches("0x"),
        )?
        .try_into()
        .map_err(|_| anyhow!("chain_hash must contain 32 bytes"))?;
        let schema_json = &schema["schema"];
        let schema = Schema::from_json(&schema_json.to_string())?;
        let signing_root = schema.rollup_expected_index(RollupRoots::TransactionSigningPayload)?;
        let legacy =
            schema_json["types"][signing_root]["Struct"]["type_name"] == "UnsignedTransaction";
        ensure!(
            legacy
                || schema_json["types"][signing_root]["Enum"]["type_name"]
                    == "TransactionSigningPayload",
            "unsupported signing schema"
        );
        let mut details = json!({
            "max_priority_fee_bips": 0,
            "max_fee": config.max_fee.parse::<u128>()?.to_string(),
            "gas_limit": null,
        });
        if legacy {
            details["chain_id"] = schema_json["chain_data"]["chain_id"].clone();
        } else {
            details["chain_hash_fragment"] = json!(chain_hash_fragment(&chain_hash));
        }
        let feeder = Self {
            details,
            config,
            accounts,
            chain_hash,
            schema,
            legacy,
        };
        feeder
            .encode(RollupRoots::RuntimeCall, &feeder.config.runtime_call)
            .context("runtime_call does not match the target schema")?;
        Ok(feeder)
    }

    fn encode(&self, root: RollupRoots, value: &Value) -> Result<Vec<u8>> {
        Ok(self
            .schema
            .json_to_borsh(self.schema.rollup_expected_index(root)?, &value.to_string())?)
    }

    fn account(&self, ctx: &Value) -> Result<&Account> {
        let vu = ctx["vu"].as_u64().context("missing vu")?;
        self.accounts
            .get(
                usize::try_from(vu)?
                    .checked_sub(1)
                    .context("VU IDs start at 1")?,
            )
            .context("VU out of range")
    }

    fn row(&self, ctx: Value) -> Result<Value> {
        let account = self.account(&ctx)?;
        match ctx["request"].as_str() {
            Some("nonce") => {
                *account.nonce.lock().unwrap() = None;
                Ok(json!({"row": {"url": account.nonce_url}}))
            }
            Some("transfer") => {
                let nonce = account
                    .nonce
                    .lock()
                    .unwrap()
                    .take()
                    .context("nonce lookup must succeed before transfer")?;
                let mut tx = json!({
                    "runtime_call": self.config.runtime_call,
                    "uniqueness": {"nonce": nonce},
                    "details": self.details,
                });
                let bytes = if self.legacy {
                    let mut bytes = self.encode(RollupRoots::TransactionSigningPayload, &tx)?;
                    bytes.extend_from_slice(&self.chain_hash);
                    bytes
                } else {
                    tx["address_override"] = Value::Null;
                    let mut payload = tx.clone();
                    payload["chain_hash"] = json!(self.chain_hash);
                    self.encode(
                        RollupRoots::TransactionSigningPayload,
                        &json!({"V0": payload}),
                    )?
                };
                tx["pub_key"] = json!(hex::encode(account.key.public_key()));
                tx["signature"] = json!(hex::encode(account.key.sign(&bytes)?));
                Ok(json!({"row": {
                    "url": format!("{}/sequencer/txs", self.config.url.trim_end_matches('/')),
                    "body": json!({"body": STANDARD.encode(self.encode(RollupRoots::Transaction, &json!({"V0": tx}))?)}).to_string(),
                }}))
            }
            _ => anyhow::bail!("use sequential requests named 'nonce' and 'transfer'"),
        }
    }

    fn result(&self, result: Value) -> Result<()> {
        if result["request"] == "nonce" {
            let account = self.account(&result)?;
            let response = &result["response"];
            let mut nonce = account.nonce.lock().unwrap();
            *nonce = None;
            if response["status"] == 200 && response["error"].is_null() {
                let body: Value = serde_json::from_str(
                    response["body"]
                        .as_str()
                        .context("missing nonce response")?,
                )?;
                *nonce = Some(body["nonce"].as_u64().context("invalid nonce response")?);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Source(Option<Feeder>);

impl FfiDataSource for Source {
    fn name(&self) -> RString {
        "sov-bank".into()
    }

    fn init(&mut self, input: RString) -> RResult<(), RString> {
        let result = (|| -> Result<()> {
            let init: Init = serde_json::from_str(&input)?;
            ensure!(
                init.sources.len() == 1,
                "use exactly one plugin data source"
            );
            let url = format!(
                "{}/rollup/schema",
                init.sources
                    .values()
                    .next()
                    .unwrap()
                    .url
                    .trim_end_matches('/')
            );
            let schema = std::thread::spawn(move || -> Result<Value> {
                Ok(reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()?
                    .get(url)
                    .send()?
                    .error_for_status()?
                    .json()?)
            })
            .join()
            .map_err(|_| anyhow!("schema fetch thread panicked"))??;
            self.0 = Some(Feeder::new(init, schema)?);
            Ok(())
        })();
        result.map_err(|e| RString::from(format!("{e:#}"))).into()
    }

    fn next_row(&self, input: RString) -> RResult<RString, RString> {
        (|| -> Result<RString> {
            let feeder = self.0.as_ref().context("plugin was not initialized")?;
            Ok(feeder
                .row(serde_json::from_str(&input)?)?
                .to_string()
                .into())
        })()
        .map_err(|e| RString::from(format!("{e:#}")))
        .into()
    }

    fn wants_results(&self) -> bool {
        true
    }

    fn on_result(&self, input: RString) {
        let result = (|| -> Result<()> {
            self.0
                .as_ref()
                .context("plugin was not initialized")?
                .result(serde_json::from_str(&input)?)
        })();
        if let Err(error) = result {
            eprintln!("sov-bank: {error:#}");
        }
    }
}

extern "C" fn info() -> RString {
    json!({
        "name": "sov-bank", "version": env!("CARGO_PKG_VERSION"), "kind": "service",
        "description": "Schema-driven Sovereign transactions with nonce discovery",
    })
    .to_string()
    .into()
}

extern "C" fn source() -> FfiDataSourceBox {
    FfiDataSource_TO::from_value(Source::default(), abi_stable::erased_types::TD_Opaque)
}

loadr_plugin_api::export_loadr_plugin! {
    PluginMod {
        abi_version: LOADR_PLUGIN_ABI_VERSION,
        info,
        make_output: RNone,
        make_protocol: RNone,
        make_service: RNone,
        make_data_source: RSome(source),
    }
}

#[cfg(test)]
mod tests;
