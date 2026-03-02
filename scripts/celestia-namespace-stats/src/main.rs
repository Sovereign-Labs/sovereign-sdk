use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{ensure, Context};
use celestia_types::nmt::{Namespace, NS_ID_V0_SIZE};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "celestia-namespace-stats")]
#[command(about = "Scans a Celestia namespace and writes per-block blob statistics as CSV")]
struct Cli {
    /// Celestia JSON-RPC URL (for example: ws://localhost:26658 or wss://...).
    #[arg(long)]
    rpc_url: String,

    /// Namespace to scan.
    /// ASCII values are interpreted directly as v0 namespace IDs.
    /// Hex values must be prefixed with 0x (for example: 0x0000736f762d74657374).
    #[arg(long)]
    namespace: String,

    /// First block height to scan (inclusive).
    #[arg(long)]
    start_height: u64,

    /// Last block height to scan (inclusive).
    #[arg(long)]
    end_height: u64,

    /// Optional output path. If omitted, CSV is written to stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional RPC auth token header value.
    /// If omitted, SOV_CELESTIA_RPC_AUTH_TOKEN is used when present.
    #[arg(long)]
    rpc_auth_token: Option<String>,

    /// Per-request timeout in seconds.
    #[arg(long, default_value_t = 8)]
    request_timeout_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    ensure!(
        cli.start_height <= cli.end_height,
        "--start-height must be <= --end-height"
    );
    ensure!(
        cli.request_timeout_secs > 0,
        "--request-timeout-secs must be > 0"
    );

    let namespace = parse_namespace(&cli.namespace)?;
    let rpc_auth_token = cli
        .rpc_auth_token
        .or_else(|| std::env::var("SOV_CELESTIA_RPC_AUTH_TOKEN").ok());

    let mut client_builder = celestia_client::Client::builder()
        .rpc_url(&cli.rpc_url)
        .timeout(Duration::from_secs(cli.request_timeout_secs));

    if let Some(token) = rpc_auth_token {
        client_builder = client_builder.rpc_auth_token(&token);
    }

    let client = client_builder
        .build()
        .await
        .context("failed to initialize celestia JSON-RPC client")?;

    let mut writer = make_writer(cli.output.as_deref())?;
    writeln!(
        writer,
        "height,blob_count,total_blob_size_bytes,blob_sizes_bytes"
    )?;

    for height in cli.start_height..=cli.end_height {
        let blobs = client
            .blob()
            .get_all(height, std::slice::from_ref(&namespace))
            .await
            .with_context(|| format!("failed to fetch namespace blobs at height {height}"))?
            .unwrap_or_default();

        let mut total_blob_size_bytes = 0_u64;
        let mut blob_sizes_bytes = Vec::with_capacity(blobs.len());
        for blob in blobs {
            let blob_size = u64::try_from(blob.data.len())
                .context("blob size does not fit into u64 on this platform")?;
            total_blob_size_bytes = total_blob_size_bytes
                .checked_add(blob_size)
                .context("total blob size overflowed u64")?;
            blob_sizes_bytes.push(blob_size);
        }

        let blob_count = u64::try_from(blob_sizes_bytes.len())
            .context("blob count does not fit into u64 on this platform")?;
        let blob_sizes_csv = blob_sizes_bytes
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(";");

        writeln!(
            writer,
            "{height},{blob_count},{total_blob_size_bytes},{blob_sizes_csv}"
        )?;
        tokio::time::sleep(Duration::from_millis(10)).await; // Limit to less than 100 blocks per
                                                             // second to avoid overloading node
    }

    writer.flush().context("failed to flush CSV output")?;
    Ok(())
}

fn make_writer(path: Option<&Path>) -> anyhow::Result<Box<dyn Write>> {
    if let Some(path) = path {
        let file = File::create(path)
            .with_context(|| format!("failed to create output file: {}", path.display()))?;
        return Ok(Box::new(BufWriter::new(file)));
    }

    Ok(Box::new(BufWriter::new(io::stdout())))
}

fn parse_namespace(input: &str) -> anyhow::Result<Namespace> {
    let namespace = input.trim();
    ensure!(!namespace.is_empty(), "--namespace cannot be empty");

    if let Some(namespace_hex) = namespace
        .strip_prefix("0x")
        .or_else(|| namespace.strip_prefix("0X"))
    {
        let namespace_bytes =
            hex::decode(namespace_hex).context("failed to decode --namespace as hex")?;
        return Namespace::new_v0(&namespace_bytes).with_context(|| {
            format!("invalid v0 namespace hex: expected at most {NS_ID_V0_SIZE} decoded bytes")
        });
    }

    Namespace::new_v0(namespace.as_bytes()).with_context(|| {
        format!(
            "invalid ASCII namespace: expected at most {NS_ID_V0_SIZE} bytes, or use 0x-prefixed hex"
        )
    })
}

#[cfg(test)]
mod tests {
    use celestia_types::nmt::Namespace;

    use super::parse_namespace;

    #[test]
    fn parses_ascii_namespace() {
        let parsed = parse_namespace("sov-test").expect("valid namespace");
        let expected = Namespace::new_v0(b"sov-test").expect("valid namespace");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parses_hex_namespace() {
        let parsed = parse_namespace("0x736f762d74657374").expect("valid namespace");
        let expected = Namespace::new_v0(b"sov-test").expect("valid namespace");
        assert_eq!(parsed, expected);
    }

    #[test]
    fn rejects_empty_namespace() {
        assert!(parse_namespace(" ").is_err());
    }
}
