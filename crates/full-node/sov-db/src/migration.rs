//! Offline migration of a namespace's NOMT page-table (`hashtable_buckets`) to a
//! different bucket count, **without replaying DA**.
//!
//! `hashtable_buckets` is fixed at DB creation and cannot be changed in place
//! (`nomt` has no in-place resize), yet the kernel keyspace grows over a chain's
//! lifetime, so a long-running node can outgrow its page table. The only documented
//! remedy ("resync database") means replaying DA, which is impossible for a node
//! without archival DA.
//!
//! This rebuilds the trie purely from the **flat** state store, which is independent
//! of the bucket count: it enumerates `(key, value)` from the namespace's flat column
//! family, re-derives each authenticated leaf, and commits them into a fresh NOMT
//! opened with the new bucket count. The rebuilt root is asserted equal to the source
//! root **before anything is persisted**, so a faulty reconstruction aborts instead of
//! corrupting state.
//!
//! Run it with the node **stopped** (so the flat store and the source NOMT are
//! consistent), then swap the rebuilt directory in and set `kernel_hashtable_buckets`
//! / `user_hashtable_buckets` in the config to the new value before restarting.

use std::path::Path;

use nomt::hasher::BinaryHasher;
use nomt::trie::KeyPath;
use nomt::{KeyReadWrite, Nomt, Options, SessionParams};
use sov_rollup_interface::reexports::digest::{self, Digest};

use crate::namespaces::{KernelNamespace, Namespace, UserNamespace};

/// Directory suffix of the flat state DB (mirrors `flat_db.rs` `DB_PATH_SUFFIX`).
const FLAT_STATE_DB_SUFFIX: &str = "state-db";
/// NOMT directory for the kernel namespace (mirrors `config.rs`).
const KERNEL_NOMT_DIR: &str = "kernel_nomt_db";
/// NOMT directory for the user namespace (mirrors `config.rs`).
const USER_NOMT_DIR: &str = "user_nomt_db";

/// Outcome of a successful migration run.
#[derive(Debug)]
pub struct MigrationReport {
    /// Number of flat entries migrated.
    pub entries: u64,
    /// Bucket count of the rebuilt page table.
    pub new_buckets: u32,
    /// The (matching) state root, as `nomt` prints it.
    pub root: String,
}

/// Re-derive the authenticated leaf value committed into the trie for a flat value.
///
/// Mirrors `sov_state`'s `NodeLeaf::make_leaf` + `combine_val_hash_and_size`: the
/// 32-byte value hash followed by the value length as a little-endian `u32`.
/// Correctness is not assumed — it is enforced by the root-equality gate in
/// [`rebuild`].
fn leaf_commitment<H: Digest<OutputSize = digest::typenum::U32>>(value: &[u8]) -> Vec<u8> {
    let mut out = H::digest(value).to_vec();
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out
}

fn make_options(path: &Path, buckets: u32, commit_concurrency: usize) -> Options {
    let mut o = Options::new();
    o.metrics(true);
    o.rollback(true);
    o.max_rollback_log_len(1);
    o.commit_concurrency(commit_concurrency);
    o.hashtable_buckets(buckets);
    o.path(path.to_path_buf());
    o
}

/// Rebuild the **kernel** namespace NOMT with `new_buckets` into `out_dir`.
///
/// `data_dir` is the rollup storage path (the directory that contains
/// `kernel_nomt_db` and `state-db`). `source_buckets` is the bucket count the
/// existing DB was created with (the default is `256_000` if it was never set).
pub fn rebuild_kernel<H>(
    data_dir: &Path,
    source_buckets: u32,
    new_buckets: u32,
    out_dir: &Path,
    commit_concurrency: usize,
) -> anyhow::Result<MigrationReport>
where
    H: Digest<OutputSize = digest::typenum::U32> + Send + Sync + 'static,
{
    rebuild::<H>(
        data_dir,
        KERNEL_NOMT_DIR,
        KernelNamespace::STATE_VALUES_TABLE_NAME,
        source_buckets,
        new_buckets,
        out_dir,
        commit_concurrency,
    )
}

/// Rebuild the **user** namespace NOMT with `new_buckets` into `out_dir`.
pub fn rebuild_user<H>(
    data_dir: &Path,
    source_buckets: u32,
    new_buckets: u32,
    out_dir: &Path,
    commit_concurrency: usize,
) -> anyhow::Result<MigrationReport>
where
    H: Digest<OutputSize = digest::typenum::U32> + Send + Sync + 'static,
{
    rebuild::<H>(
        data_dir,
        USER_NOMT_DIR,
        UserNamespace::STATE_VALUES_TABLE_NAME,
        source_buckets,
        new_buckets,
        out_dir,
        commit_concurrency,
    )
}

#[allow(clippy::too_many_arguments)]
fn rebuild<H>(
    data_dir: &Path,
    nomt_dir: &str,
    cf_name: &str,
    source_buckets: u32,
    new_buckets: u32,
    out_dir: &Path,
    commit_concurrency: usize,
) -> anyhow::Result<MigrationReport>
where
    H: Digest<OutputSize = digest::typenum::U32> + Send + Sync + 'static,
{
    // 1. Source root (the value the rebuild must reproduce).
    let source = Nomt::<BinaryHasher<H>>::open(make_options(
        &data_dir.join(nomt_dir),
        source_buckets,
        commit_concurrency,
    ))?;
    let source_root = format!("{:?}", source.root());

    // 2. Enumerate the namespace's flat values (independent of bucket count). The flat
    //    key/value bytes are the canonical `SlotKey`/`SlotValue` encodings.
    let statedb = data_dir.join(FLAT_STATE_DB_SUFFIX);
    let cfs = rockbound::rocksdb::DB::list_cf(&rockbound::rocksdb::Options::default(), &statedb)?;
    let db = rockbound::rocksdb::DB::open_cf_for_read_only(
        &rockbound::rocksdb::Options::default(),
        &statedb,
        &cfs,
        false,
    )?;
    let cf = db
        .cf_handle(cf_name)
        .ok_or_else(|| anyhow::anyhow!("flat column family `{cf_name}` not found"))?;

    let mut writes: Vec<(KeyPath, KeyReadWrite)> = Vec::new();
    for item in db.iterator_cf(&cf, rockbound::rocksdb::IteratorMode::Start) {
        let (k, v) = item?;
        let key_path: KeyPath = H::digest(&k).into();
        writes.push((
            key_path,
            KeyReadWrite::Write(Some(leaf_commitment::<H>(&v))),
        ));
    }
    let entries = writes.len() as u64;
    writes.sort_by(|a, b| a.0.cmp(&b.0));
    writes.dedup_by(|a, b| a.0 == b.0);

    // 3. Commit into a fresh NOMT with the new bucket count and verify the root.
    let out =
        Nomt::<BinaryHasher<H>>::open(make_options(out_dir, new_buckets, commit_concurrency))?;
    let session = out.begin_session(SessionParams::default());
    let finished = session.finish(writes)?;
    let new_root = format!("{:?}", finished.root());
    anyhow::ensure!(
        new_root == source_root,
        "root mismatch — refusing to persist. source={source_root} rebuilt={new_root}"
    );
    finished.commit(&out)?;

    Ok(MigrationReport {
        entries,
        new_buckets,
        root: new_root,
    })
}
