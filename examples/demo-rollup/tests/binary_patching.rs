use std::ffi::OsString;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use serde_json::{json, Value};
use sov_chain_config_patcher::{build_bundle, PatchOptions};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Instant};

const PATCHED_CHAIN_ID: u64 = 4_321_001;
const PATCHED_CHAIN_NAME: &str = "PatchedDemoChain";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(90);
const SP1_ENABLED: bool = cfg!(all(not(skip_guest_build), debug_assertions));
const TEST_SP1_INNER_ELF_ENV: &str = "SOV_TEST_SP1_GUEST_MOCK_ELF";
const TEST_SP1_OUTER_ELF_ENV: &str = "SOV_TEST_SP1_GUEST_AGGREGATION_MOCK_ELF";

#[tokio::test(flavor = "multi_thread")]
async fn patched_node_and_cli_use_the_new_chain_identity() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .and_then(Path::parent)
        .context("demo-rollup is not nested under the workspace root")?;

    let constants_path = temp.path().join("constants.toml");
    write_patched_constants(&workspace_dir.join("constants.toml"), &constants_path)?;

    // Test-profile binaries contain several GiB of debug info. Production binaries do not, and
    // copying and hashing that unrelated data would make this test prohibitively slow.
    let patch_inputs = temp.path().join("patch-inputs");
    fs::create_dir(&patch_inputs)?;
    let node_input = patch_inputs.join("sov-demo-rollup");
    let cli_input = patch_inputs.join("sov-cli");
    strip_debug_info(
        Path::new(env!("CARGO_BIN_EXE_sov-demo-rollup")),
        &node_input,
    )
    .await?;
    strip_debug_info(Path::new(env!("CARGO_BIN_EXE_sov-cli")), &cli_input).await?;

    let (sp1_inner_elf, sp1_outer_elf) = sp1_patch_inputs(&manifest_dir);
    let bundle_dir = temp.path().join("bundle");
    let patch_options = PatchOptions {
        native_binaries: vec![node_input, cli_input],
        sp1_inner_elf,
        sp1_outer_elf,
        constants_toml: constants_path,
        output_dir: bundle_dir.clone(),
    };
    let patch_manifest = tokio::task::spawn_blocking(move || build_bundle(&patch_options))
        .await
        .context("chain-config patch task panicked")??;
    assert_sp1_manifest_presence(&patch_manifest)?;
    let config_path = temp.path().join("rollup.toml");
    let port = unused_port()?;
    write_test_rollup_config(
        &manifest_dir.join("configs/mock_rollup_config.toml"),
        &config_path,
        temp.path(),
        port,
    )?;
    let genesis_dir = temp.path().join("genesis");
    copy_genesis_dir(
        &workspace_dir.join("examples/test-data/genesis/demo/mock"),
        &genesis_dir,
    )?;

    let log_path = temp.path().join("rollup.log");
    let mut child = spawn_rollup(
        &bundle_dir.join("native/sov-demo-rollup"),
        &config_path,
        &genesis_dir,
        &log_path,
        "mock",
        None,
        false,
    )?;
    let base_url = format!("http://127.0.0.1:{port}");

    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let constants = wait_for_json(
            &client,
            &format!("{base_url}/rollup/constants"),
            &mut child,
            |_| true,
        )
        .await?;
        ensure!(
            constants["chain_id"] == PATCHED_CHAIN_ID,
            "constants endpoint returned the wrong chain ID: {constants}"
        );
        ensure!(
            constants["chain_name"] == PATCHED_CHAIN_NAME,
            "constants endpoint returned the wrong chain name: {constants}"
        );

        let schema: Value = client
            .get(format!("{base_url}/rollup/schema"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(schema["chain_hash"] == patch_manifest.chain.chain_hash.to_string());

        let chain_id = rpc_result(&client, &base_url, "eth_chainId", json!([])).await?;
        ensure!(chain_id == format!("0x{PATCHED_CHAIN_ID:x}"));

        let network_id = rpc_result(&client, &base_url, "net_version", json!([])).await?;
        ensure!(network_id == PATCHED_CHAIN_ID.to_string());

        let target = "0x0000000000000000000000000000000000000042";
        let state_overrides = json!({
            (target): {
                // CHAINID; MSTORE(0); RETURN(0, 32)
                "code": "0x4660005260206000f3",
            }
        });
        let simulated_chain_id = rpc_result(
            &client,
            &base_url,
            "eth_call",
            json!([{"to": target}, "latest", state_overrides.clone()]),
        )
        .await?;
        ensure!(
            simulated_chain_id == format!("0x{PATCHED_CHAIN_ID:064x}"),
            "eth_call CHAINID result did not use the patched chain ID"
        );

        let estimated_gas = rpc_result(
            &client,
            &base_url,
            "eth_estimateGas",
            json!([
                {
                    "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "to": target,
                },
                "latest",
                state_overrides,
            ]),
        )
        .await?;
        let estimated_gas = estimated_gas
            .as_str()
            .context("eth_estimateGas result is not a hex quantity")?;
        ensure!(
            u64::from_str_radix(estimated_gas.trim_start_matches("0x"), 16)? > 0,
            "eth_estimateGas returned zero gas"
        );

        wait_for_json(
            &client,
            &format!("{base_url}/ledger/slots/finalized"),
            &mut child,
            |slot| slot["number"].as_u64().is_some_and(|number| number >= 1),
        )
        .await?;

        submit_transaction_with_patched_cli(
            &bundle_dir.join("native/sov-cli"),
            &base_url,
            temp.path(),
            workspace_dir,
        )
        .await
    }
    .await;

    let _ = child.kill().await;
    if let Err(error) = result {
        let logs = fs::read_to_string(&log_path).unwrap_or_default();
        return Err(error.context(format!("patched rollup log:\n{logs}")));
    }

    if SP1_ENABLED {
        verify_sp1_bundle(
            &bundle_dir,
            &patch_manifest,
            temp.path(),
            &manifest_dir,
            &genesis_dir,
        )
        .await?;
    }

    Ok(())
}

fn sp1_patch_inputs(manifest_dir: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    if SP1_ENABLED {
        let sp1_dir = manifest_dir.join("provers/sp1");
        (
            Some(sp1_dir.join(
                "guest-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-mock-sp1",
            )),
            Some(sp1_dir.join(
                "guest-aggregation-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-aggregated-proof-program",
            )),
        )
    } else {
        (None, None)
    }
}

fn assert_sp1_manifest_presence(
    manifest: &sov_chain_config_patcher::PatchManifest,
) -> anyhow::Result<()> {
    ensure!(
        manifest.sp1.is_some() == SP1_ENABLED,
        "SP1 manifest presence does not match whether SP1 patching is enabled"
    );
    Ok(())
}

async fn verify_sp1_bundle(
    bundle_dir: &Path,
    patch_manifest: &sov_chain_config_patcher::PatchManifest,
    temp: &Path,
    manifest_dir: &Path,
    genesis_dir: &Path,
) -> anyhow::Result<()> {
    let sp1_temp = temp.join("sp1-node");
    fs::create_dir(&sp1_temp)?;
    let config_path = sp1_temp.join("rollup.toml");
    let port = unused_port()?;
    write_test_rollup_config(
        &manifest_dir.join("configs/mock_rollup_config.toml"),
        &config_path,
        &sp1_temp,
        port,
    )?;

    let inner_elf = bundle_dir.join("sp1/inner.elf");
    let outer_elf = bundle_dir.join("sp1/outer.elf");
    let log_path = sp1_temp.join("rollup.log");
    let mut child = spawn_rollup(
        &bundle_dir.join("native/sov-demo-rollup"),
        &config_path,
        genesis_dir,
        &log_path,
        "sp1",
        Some((&inner_elf, &outer_elf)),
        true,
    )?;
    let base_url = format!("http://127.0.0.1:{port}");

    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let constants = wait_for_json(
            &client,
            &format!("{base_url}/rollup/constants"),
            &mut child,
            |_| true,
        )
        .await?;
        ensure!(
            constants["chain_id"] == PATCHED_CHAIN_ID,
            "constants endpoint returned the wrong chain ID: {constants}"
        );
        ensure!(
            constants["chain_name"] == PATCHED_CHAIN_NAME,
            "constants endpoint returned the wrong chain name: {constants}"
        );

        let sp1 = patch_manifest
            .sp1
            .as_ref()
            .context("patch manifest has no SP1 commitments")?;
        let inner_artifact = patch_manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.output == Path::new("sp1/inner.elf"))
            .context("patch manifest has no SP1 inner artifact")?;
        ensure!(inner_artifact.patched, "SP1 inner ELF was not patched");
        ensure!(
            inner_artifact.sha256_before != inner_artifact.sha256_after,
            "SP1 inner ELF hash did not change"
        );
        let chain_state: Value =
            serde_json::from_slice(&fs::read(genesis_dir.join("chain_state.json"))?)?;
        ensure!(
            chain_state["inner_code_commitment"] == json!(sp1.inner_code_commitment.words),
            "SP1 inner commitment override disagrees with patch manifest"
        );
        ensure!(
            chain_state["outer_code_commitment"] == json!(sp1.outer_code_commitment.words),
            "SP1 outer commitment override disagrees with patch manifest"
        );
        ensure!(!sp1.outer_patched, "SP1 outer ELF should not be patched");
        Ok(())
    }
    .await;

    let _ = child.kill().await;
    if let Err(error) = result {
        let logs = fs::read_to_string(&log_path).unwrap_or_default();
        return Err(error.context(format!("patched SP1 rollup log:\n{logs}")));
    }
    Ok(())
}

async fn strip_debug_info(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let output = Command::new("objcopy")
        .arg("--strip-debug")
        .arg(source)
        .arg(destination)
        .output()
        .await
        .context("failed to run objcopy")?;
    ensure!(
        output.status.success(),
        "objcopy failed to strip {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn write_patched_constants(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let source = fs::read_to_string(source)?;
    let mut chain_id_replaced = false;
    let mut chain_name_replaced = false;
    let mut output = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("CHAIN_ID =") {
            output.push_str(&format!("CHAIN_ID = {PATCHED_CHAIN_ID}"));
            chain_id_replaced = true;
        } else if trimmed.starts_with("CHAIN_NAME =") {
            output.push_str(&format!("CHAIN_NAME = \"{PATCHED_CHAIN_NAME}\""));
            chain_name_replaced = true;
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    ensure!(
        chain_id_replaced,
        "constants.toml has no CHAIN_ID assignment"
    );
    ensure!(
        chain_name_replaced,
        "constants.toml has no CHAIN_NAME assignment"
    );
    fs::write(destination, output)?;
    Ok(())
}

fn write_test_rollup_config(
    source: &Path,
    destination: &Path,
    temp: &Path,
    port: u16,
) -> anyhow::Result<()> {
    let source = fs::read_to_string(source)?;
    let replacements = [
        (
            "connection_string",
            format!(
                "connection_string = \"sqlite://{}?mode=rwc\"",
                temp.join("mock-da.sqlite").display()
            ),
        ),
        (
            "path",
            format!("path = \"{}\"", temp.join("storage").display()),
        ),
        ("bind_port", format!("bind_port = {port}")),
        ("state_cache_size", "state_cache_size = 67108864".to_owned()),
        (
            "user_hashtable_buckets",
            "user_hashtable_buckets = 10000".to_owned(),
        ),
    ];
    let mut replaced = vec![false; replacements.len()];
    let mut output = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some((index, (_, replacement))) = replacements
            .iter()
            .enumerate()
            .find(|(_, (key, _))| trimmed.starts_with(&format!("{key} =")))
        {
            output.push_str(replacement);
            replaced[index] = true;
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    ensure!(
        replaced.into_iter().all(|value| value),
        "rollup config is missing a field required by the patch test"
    );
    fs::write(destination, output)?;
    Ok(())
}

fn copy_genesis_dir(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file(),
            "genesis directory is not flat"
        );
        fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn unused_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn spawn_rollup(
    binary: &Path,
    config: &Path,
    genesis: &Path,
    log_path: &Path,
    zk_vm: &str,
    sp1_elfs: Option<(&Path, &Path)>,
    override_code_commitments: bool,
) -> anyhow::Result<Child> {
    let log = fs::File::create(log_path)?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(binary);
    command
        .args([
            "--da-layer",
            "mock",
            "--zk-vm",
            zk_vm,
            "--rollup-config-path",
        ])
        .arg(config)
        .arg("--genesis-config-dir")
        .arg(genesis)
        .env_remove("SOV_PROVER_MODE")
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true);
    if let Some((inner, outer)) = sp1_elfs {
        command
            .env(TEST_SP1_INNER_ELF_ENV, inner)
            .env(TEST_SP1_OUTER_ELF_ENV, outer);
    }
    if override_code_commitments {
        command.arg("--override-code-commitments");
    }
    command
        .spawn()
        .context("failed to start patched demo-rollup")
}

async fn wait_for_json<F>(
    client: &reqwest::Client,
    url: &str,
    child: &mut Child,
    is_ready: F,
) -> anyhow::Result<Value>
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut last_error = None;
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("patched demo-rollup exited before becoming ready: {status}");
        }
        // Connection, status, and body failures are all expected while the node is starting, so
        // retain the latest one for the eventual timeout diagnostic and keep polling.
        match client.get(url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.json().await {
                    Ok(value) => {
                        if is_ready(&value) {
                            return Ok(value);
                        }
                    }
                    Err(error) => last_error = Some(error.to_string()),
                },
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error.to_string()),
        }
        if Instant::now() >= deadline {
            match last_error {
                Some(error) => bail!("patched demo-rollup startup timed out: {error}"),
                None => bail!("patched demo-rollup startup timed out"),
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn rpc_result(
    client: &reqwest::Client,
    base_url: &str,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let response: Value = client
        .post(format!("{base_url}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        response.get("error").is_none(),
        "{method} returned an RPC error: {response}"
    );
    response
        .get("result")
        .cloned()
        .with_context(|| format!("{method} response has no result: {response}"))
}

async fn submit_transaction_with_patched_cli(
    cli: &Path,
    base_url: &str,
    temp: &Path,
    workspace: &Path,
) -> anyhow::Result<()> {
    let wallet = temp.join("wallet");
    fs::create_dir(&wallet)?;
    run_cli(
        cli,
        &wallet,
        vec!["node".into(), "set-url".into(), base_url.into()],
    )
    .await?;
    run_cli(
        cli,
        &wallet,
        vec![
            "keys".into(),
            "import".into(),
            "--nickname".into(),
            "PATCH_TEST".into(),
            "--path".into(),
            workspace
                .join("examples/test-data/keys/token_deployer_private_key.json")
                .into_os_string(),
        ],
    )
    .await?;
    run_cli(
        cli,
        &wallet,
        vec![
            "transactions".into(),
            "import".into(),
            "from-file".into(),
            "bank".into(),
            "--chain-id".into(),
            PATCHED_CHAIN_ID.to_string().into(),
            "--max-fee".into(),
            "100000000".into(),
            "--path".into(),
            workspace
                .join("examples/test-data/requests/create_token.json")
                .into_os_string(),
        ],
    )
    .await?;
    run_cli(
        cli,
        &wallet,
        vec![
            "node".into(),
            "submit-batch".into(),
            "--wait-for-processing".into(),
            "by-nickname".into(),
            "PATCH_TEST".into(),
        ],
    )
    .await
}

async fn run_cli(binary: &Path, wallet: &Path, args: Vec<OsString>) -> anyhow::Result<()> {
    let output = timeout(
        PROCESS_TIMEOUT,
        Command::new(binary)
            .args(args)
            .env("SOV_WALLET_DIR", wallet)
            .output(),
    )
    .await
    .context("patched sov-cli timed out")??;
    ensure!(
        output.status.success(),
        "patched sov-cli failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
