//! Offline tool to rebuild a namespace's NOMT page-table with a new
//! `hashtable_buckets`, without replaying DA. Run with the node **stopped**.
//!
//! ```text
//! cargo run -p sov-db --example migrate_nomt_buckets -- \
//!     --data-dir /path/to/rollup/data \
//!     --namespace kernel \
//!     --source-buckets 256000 \
//!     --new-buckets 4000000 \
//!     --out /path/to/rollup/data/kernel_nomt_db.new
//! ```
//!
//! The rebuilt root is verified equal to the source root before anything is
//! persisted. On success, stop the node, swap `--out` in for the live
//! `<namespace>_nomt_db`, set the matching `*_hashtable_buckets` in the config to
//! `--new-buckets`, and restart.
//!
//! NOTE: the hasher is fixed to `sha2::Sha256` here, which matches the default
//! `CryptoSpec::Hasher`. If your rollup uses a different state hasher, adjust it.

use std::path::PathBuf;

use sha2::Sha256;
use sov_db::migration::{rebuild_kernel, rebuild_user};

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let data_dir = PathBuf::from(
        arg(&args, "--data-dir").ok_or_else(|| anyhow::anyhow!("missing --data-dir"))?,
    );
    let namespace = arg(&args, "--namespace")
        .ok_or_else(|| anyhow::anyhow!("missing --namespace (kernel|user)"))?;
    let new_buckets: u32 = arg(&args, "--new-buckets")
        .ok_or_else(|| anyhow::anyhow!("missing --new-buckets"))?
        .parse()?;
    let out = PathBuf::from(arg(&args, "--out").ok_or_else(|| anyhow::anyhow!("missing --out"))?);
    // The bucket count the existing DB was created with. Defaults to the sov-db
    // kernel default (256_000); pass explicitly for the user namespace.
    let source_buckets: u32 = arg(&args, "--source-buckets")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(256_000);
    let commit_concurrency: usize = arg(&args, "--commit-concurrency")
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(2);

    let report = match namespace.as_str() {
        "kernel" => rebuild_kernel::<Sha256>(
            &data_dir,
            source_buckets,
            new_buckets,
            &out,
            commit_concurrency,
        )?,
        "user" => rebuild_user::<Sha256>(
            &data_dir,
            source_buckets,
            new_buckets,
            &out,
            commit_concurrency,
        )?,
        other => anyhow::bail!("unknown --namespace `{other}` (expected kernel|user)"),
    };

    println!(
        "migrated {} entries into {} buckets; verified root {} — wrote {}",
        report.entries,
        report.new_buckets,
        report.root,
        out.display()
    );
    Ok(())
}
