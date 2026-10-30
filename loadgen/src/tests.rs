use super::*;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use std::io::Write;

const DEMO_SCHEMA: &str = include_str!("../../examples/demo-rollup/demo-rollup-schema.json");
const RECIPIENT: &str = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf";
const TOKEN: &str = "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7";

fn config(keys: &std::path::Path) -> Value {
    json!({
        "url": "http://127.0.0.1:12346", "keys_path": keys, "crypto": "ed25519",
        "max_fee": "100000000",
        "runtime_call": {"bank": {"transfer": {
            "to": {"Standard": RECIPIENT}, "coins": {"amount": "1", "token_id": TOKEN},
        }}},
    })
}

fn init(keys: &[String], offset: usize, vus: usize) -> (Init, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    serde_json::to_writer(&mut file, keys).unwrap();
    let init = serde_json::from_value(json!({
        "sources": {"tx": config(file.path())}, "vus": vus, "vu_offset": offset,
    }))
    .unwrap();
    (init, file)
}

fn schema_response(schema: Value) -> Value {
    json!({"schema": schema, "chain_hash": hex::encode([7; 32])})
}

fn demo_response() -> Value {
    schema_response(serde_json::from_str(DEMO_SCHEMA).unwrap())
}

fn feeder() -> Feeder {
    let (init, _file) = init(&[hex::encode([1; 32])], 0, 1);
    Feeder::new(init, demo_response()).unwrap()
}

fn lookup(feeder: &Feeder, vu: u64, nonce: u64) {
    feeder.row(json!({"vu": vu, "request": "nonce"})).unwrap();
    feeder
        .result(json!({
            "vu": vu, "request": "nonce",
            "response": {"status": 200, "error": null, "body": json!({"nonce": nonce}).to_string()},
        }))
        .unwrap();
}

fn tx(feeder: &Feeder, vu: u64) -> Vec<u8> {
    let row = feeder
        .row(json!({"vu": vu, "request": "transfer"}))
        .unwrap();
    let body: Value = serde_json::from_str(row["row"]["body"].as_str().unwrap()).unwrap();
    STANDARD.decode(body["body"].as_str().unwrap()).unwrap()
}

#[test]
fn bank_transfer_has_valid_signature_nonce_and_active_chain_hash() {
    let feeder = feeder();
    assert_ne!(feeder.schema.chain_hash().unwrap(), feeder.chain_hash);
    lookup(&feeder, 1, 43);
    let tx = tx(&feeder, 1);
    assert_eq!(tx[0], 0);
    let signature = ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap();
    let public = ed25519_dalek::VerifyingKey::from_bytes(tx[65..97].try_into().unwrap()).unwrap();
    let mut signing = vec![0];
    signing.extend_from_slice(&tx[97..]);
    signing.extend_from_slice(&[7; 32]);
    public.verify_strict(&signing, &signature).unwrap();
    let n = tx.len();
    assert_eq!(
        u64::from_le_bytes(tx[n - 42..n - 34].try_into().unwrap()),
        43
    );
    assert_eq!(
        &tx[n - 9..n - 1],
        &chain_hash_fragment(&[7; 32]).to_le_bytes()
    );
    assert!(feeder.schema.display(0, &tx).unwrap().contains("Transfer"));
    assert!(feeder.row(json!({"vu": 1, "request": "transfer"})).is_err());
}

#[test]
fn failed_lookups_never_reuse_a_previous_nonce() {
    let feeder = feeder();
    for response in [
        json!({"status": 503, "error": null, "body": "{\"nonce\":99}"}),
        json!({"status": 200, "error": "timeout", "body": "{\"nonce\":99}"}),
        json!({"status": 200, "error": null, "body": "invalid"}),
        json!({"status": 200, "error": null, "body": "{\"nonce\":null}"}),
    ] {
        lookup(&feeder, 1, 7);
        feeder.row(json!({"vu": 1, "request": "nonce"})).unwrap();
        let _ = feeder.result(json!({"vu": 1, "request": "nonce", "response": response}));
        assert!(feeder.row(json!({"vu": 1, "request": "transfer"})).is_err());
    }
    lookup(&feeder, 1, 0);
    tx(&feeder, 1);
    lookup(&feeder, 1, u64::MAX);
    tx(&feeder, 1);
}

#[test]
fn fleet_placement_and_duplicate_keys_are_checked() {
    let keys = [hex::encode([1; 32]), hex::encode([2; 32])];
    let (a, _a_file) = init(&keys, 0, 1);
    let (b, _b_file) = init(&keys, 1, 1);
    let a = Feeder::new(a, demo_response()).unwrap();
    let b = Feeder::new(b, demo_response()).unwrap();
    assert_ne!(a.accounts[0].nonce_url, b.accounts[0].nonce_url);
    for vu in [0, 2] {
        assert!(a.row(json!({"vu": vu, "request": "nonce"})).is_err());
    }
    let (missing, _file) = init(&keys, 1, 2);
    assert!(Feeder::new(missing, demo_response()).is_err());
    let (duplicate, _file) = init(&[keys[0].clone(), keys[0].clone()], 0, 1);
    assert!(Feeder::new(duplicate, demo_response()).is_err());
    let (idle, _file) = init(&keys, 2, 0);
    assert!(Feeder::new(idle, demo_response())
        .unwrap()
        .accounts
        .is_empty());
    assert!(serde_json::from_value::<Init>(json!({"sources": {}})).is_err());
}

#[test]
fn secp256k1_matches_ethereum_credentials_and_keccak_signing() {
    let mut secret = [0; 32];
    secret[31] = 1;
    let key = Key::new(Crypto::Secp256k1, &secret).unwrap();
    assert_eq!(
        key.credential(),
        "0000000000000000000000007e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
    let public = k256::ecdsa::VerifyingKey::from_sec1_bytes(&key.public_key()).unwrap();
    let signature = k256::ecdsa::Signature::from_slice(&key.sign(b"transfer").unwrap()).unwrap();
    public
        .verify_prehash(&Keccak256::digest(b"transfer"), &signature)
        .unwrap();
    assert!(public
        .verify_prehash(&Keccak256::digest(b"other transfer"), &signature)
        .is_err());
}

#[test]
fn init_fetches_schema_and_exposes_a_nonce_row() {
    use std::{io::Read, net::TcpListener};
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = server.accept().unwrap();
        let mut buffer = [0; 4096];
        let size = socket.read(&mut buffer).unwrap();
        assert!(std::str::from_utf8(&buffer[..size])
            .unwrap()
            .starts_with("GET /rollup/schema "));
        let body = demo_response().to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let (_init, file) = init(&[hex::encode([1; 32])], 0, 1);
    let mut config = config(file.path());
    config["url"] = json!(format!("http://{address}"));
    let mut source = Source::default();
    source
        .init(
            json!({"sources": {"tx": config}, "vus": 1, "vu_offset": 0})
                .to_string()
                .into(),
        )
        .into_result()
        .unwrap();
    let row = source
        .next_row(r#"{"vu":1,"request":"nonce"}"#.into())
        .into_result()
        .unwrap();
    assert!(row.contains("/dedup?select=nonce"));
    handle.join().unwrap();
}

#[test]
#[ignore = "set SOV_LOADR_STARTER_SCHEMA to a legacy rollup-starter schema JSON file"]
fn legacy_starter_schema_produces_valid_secp256k1_transfer() {
    let path = std::env::var("SOV_LOADR_STARTER_SCHEMA").unwrap();
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let chain_id = schema["chain_data"]["chain_id"].as_u64().unwrap();
    let (mut init, _file) = init(&[hex::encode([1; 32])], 0, 1);
    let config = init.sources.get_mut("tx").unwrap();
    config.crypto = Crypto::Secp256k1;
    config.runtime_call["bank"]["transfer"]["to"] =
        json!("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
    let feeder = Feeder::new(init, schema_response(schema)).unwrap();
    assert!(feeder.legacy);
    lookup(&feeder, 1, 17);
    let tx = tx(&feeder, 1);
    assert_eq!(tx[0], 0);
    let signature = k256::ecdsa::Signature::from_slice(&tx[1..65]).unwrap();
    let public = k256::ecdsa::VerifyingKey::from_sec1_bytes(&tx[65..98]).unwrap();
    let mut signing = tx[98..].to_vec();
    signing.extend_from_slice(&[7; 32]);
    public
        .verify_prehash(&Keccak256::digest(&signing), &signature)
        .unwrap();
    let n = tx.len();
    assert_eq!(
        u64::from_le_bytes(tx[n - 41..n - 33].try_into().unwrap()),
        17
    );
    assert_eq!(&tx[n - 8..], &chain_id.to_le_bytes());
    feeder.schema.display(0, &tx).unwrap();
}

#[test]
#[ignore = "set SOV_LOADR_BIN and SOV_LOADR_PLUGIN to matching built loadr and plugin paths"]
fn loadr_executes_plan_against_verifying_endpoint() {
    use std::{
        collections::BTreeMap,
        io::Read,
        net::{TcpListener, TcpStream},
        process::Command,
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut nonces = BTreeMap::<String, u64>::new();
        let mut accepted = 0;
        loop {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            let mut request = Vec::new();
            let headers_end = loop {
                let mut buffer = [0; 4096];
                let size = socket.read(&mut buffer).unwrap();
                if size == 0 {
                    return (nonces, accepted);
                }
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(request[..headers_end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while request.len() < headers_end + length {
                let mut buffer = [0; 4096];
                let size = socket.read(&mut buffer).unwrap();
                assert_ne!(size, 0);
                request.extend_from_slice(&buffer[..size]);
            }
            let path = headers.split_whitespace().nth(1).unwrap();
            let response = if path == "/rollup/schema" {
                demo_response()
            } else if let Some(credential) = path.strip_prefix("/rollup/addresses/") {
                let credential = credential.strip_suffix("/dedup?select=nonce").unwrap();
                json!({"nonce": nonces.entry(credential.into()).or_insert(11)})
            } else {
                assert_eq!(path, "/sequencer/txs");
                let body: Value = serde_json::from_slice(&request[headers_end..]).unwrap();
                let tx = STANDARD.decode(body["body"].as_str().unwrap()).unwrap();
                let public =
                    ed25519_dalek::VerifyingKey::from_bytes(tx[65..97].try_into().unwrap())
                        .unwrap();
                let signature = ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap();
                let mut signing = vec![0];
                signing.extend_from_slice(&tx[97..]);
                signing.extend_from_slice(&[7; 32]);
                public.verify_strict(&signing, &signature).unwrap();
                let n = tx.len();
                let nonce = u64::from_le_bytes(tx[n - 42..n - 34].try_into().unwrap());
                let expected = nonces.get_mut(&hex::encode(public.to_bytes())).unwrap();
                assert_eq!(nonce, *expected);
                *expected += 1;
                accepted += 1;
                json!({"receipt": {"result": "successful"}})
            }
            .to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let (_init, keys) = init(&[hex::encode([1; 32]), hex::encode([2; 32])], 0, 2);
    let plugin = std::env::var("SOV_LOADR_PLUGIN").unwrap();
    let plan = include_str!("../bank.yaml")
        .replace("  - name: sov-bank", &format!("  - name: sov-bank\n    path: {}", json!(plugin)))
        .replace("http://127.0.0.1:12346", &format!("http://{address}"))
        .replace("/absolute/path/to/keys.json", keys.path().to_str().unwrap())
        .replace("sov1REPLACE_WITH_RECIPIENT", RECIPIENT)
        .replace("token_REPLACE_WITH_TOKEN", TOKEN)
        .replace("executor: constant-arrival-rate", "executor: per-vu-iterations\n    vus: 2\n    iterations: 5\n    max_duration: 30s")
        .replace("    rate: 100\n    time_unit: 1s\n    duration: 30s\n    pre_allocated_vus: 16\n    max_vus: 32\n", "");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("smoke.yaml");
    fs::write(&path, plan).unwrap();
    let output = Command::new(std::env::var("SOV_LOADR_BIN").unwrap())
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    drop(TcpStream::connect(address).unwrap());
    let (nonces, accepted) = handle.join().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(accepted, 10);
    assert_eq!(nonces.len(), 2);
    assert!(nonces.values().all(|n| *n == 16));
}
