//! Measures `NomtProverStorage::compute_state_update` (state-root computation during slot
//! finalization) under a bank-transfer shaped load.
//!
//! The starting state is generated once from fixed seeds and cached under
//! `FINALIZATION_BENCH_DIR` (default `<workspace>/target/finalization-bench`). Every run copies it
//! to a fresh work directory, so all runs start from identical bytes. Storage options are read from
//! the `[storage]` section of `examples/demo-rollup/configs/mock_rollup_config.toml`.
//!
//! Only `compute_state_update` is timed; materialization and commit run untimed after each slot.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::storage_manager::NomtStorageManager;
use sov_mock_da::{MockBlock, MockDaSpec, MockHash};
use sov_rollup_interface::node::da::SlotData;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{
    DefaultStorageSpec, NodeLeaf, OrderedReadsAndWrites, SlotKey, SlotValue, StateAccesses, Storage,
};

type BenchStorage = NomtProverStorage<DefaultStorageSpec<Sha256>, MockHash>;
type BenchStorageManager = NomtStorageManager<MockDaSpec, Sha256, BenchStorage>;
type Root = <BenchStorage as Storage>::Root;
type Writes = Vec<(SlotKey, Option<SlotValue>)>;

/// Bump when anything below that affects the generated starting state changes.
const SNAPSHOT_VERSION: u32 = 1;
const TIP_FILE: &str = "bench-tip.borsh";
const SEED: u64 = 0x5eed_f1a1_12a7_1000;

/// Existing accounts. Each has a balance, nonce and account key: ~10.2M user keys.
const ACCOUNTS: u32 = 3_400_000;
const KERNEL_KEYS: u32 = 10_000;
const GENESIS_BATCH: usize = 1_000_000;

const TRANSFERS_PER_SLOT: u32 = 8_850;
/// One in this many receivers is a brand new account.
const NEW_RECEIVER_EVERY: u32 = 10;
/// One in this many transfers also reads the receiver's account key.
const RECEIVER_ACCOUNT_READ_EVERY: u32 = 8;
const HOT_READ_ONLY_KEYS: u32 = 20;
const HOT_READ_WRITE_KEYS: u32 = 4;
const KERNEL_READ_WRITE_PER_SLOT: u32 = 30;
const KERNEL_READ_ONLY_PER_SLOT: u32 = 10;

const WARMUP_SLOTS: u64 = 3;
const TIMED_SLOTS: u64 = 10;

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

fn address(index: u32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SEED.to_le_bytes());
    hasher.update(index.to_le_bytes());
    hasher.finalize().into()
}

fn key(prefix: &[u8], suffix: &[u8]) -> SlotKey {
    let mut bytes = Vec::with_capacity(prefix.len() + suffix.len());
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(suffix);
    SlotKey::from_slice(&bytes)
}

fn balance_key(index: u32) -> SlotKey {
    key(b"sov_bank/Bank/balances/", &address(index))
}

fn nonce_key(index: u32) -> SlotKey {
    key(b"sov_nonces/Nonces/nonces/", &address(index))
}

fn account_key(index: u32) -> SlotKey {
    key(b"sov_accounts/Accounts/accounts/", &address(index))
}

fn hot_read_only_key(index: u32) -> SlotKey {
    key(b"sov_hot/Config/read_only/", &index.to_le_bytes())
}

fn hot_read_write_key(index: u32) -> SlotKey {
    key(b"sov_hot/Fees/accumulators/", &index.to_le_bytes())
}

fn kernel_key(index: u32) -> SlotKey {
    key(b"sov_kernel/ChainState/items/", &index.to_le_bytes())
}

fn balance_value(balance: u128) -> SlotValue {
    SlotValue::from(balance.to_le_bytes().to_vec())
}

fn nonce_value(nonce: u64) -> SlotValue {
    SlotValue::from(nonce.to_le_bytes().to_vec())
}

fn account_value(index: u32) -> SlotValue {
    let mut bytes = address(index).to_vec();
    bytes.extend_from_slice(&u64::from(index).to_le_bytes());
    SlotValue::from(bytes)
}

fn fixed_value(tag: &[u8], index: u32) -> SlotValue {
    let mut bytes = tag.to_vec();
    bytes.extend_from_slice(&index.to_le_bytes());
    SlotValue::from(bytes)
}

fn leaf(value: &SlotValue) -> Option<NodeLeaf> {
    Some(NodeLeaf::make_leaf::<Sha256>(value))
}

/// In-memory model of the values stored for the keys the load touches, so reads carry correct leaves.
struct Model {
    balances: Vec<u128>,
    nonces: Vec<u64>,
    fees: Vec<u128>,
    kernel_counters: Vec<u64>,
}

impl Model {
    fn genesis() -> Self {
        Self {
            balances: (0..ACCOUNTS)
                .map(|i| 1_000_000_000 + u128::from(i))
                .collect(),
            nonces: vec![0; ACCOUNTS as usize],
            fees: vec![0; HOT_READ_WRITE_KEYS as usize],
            kernel_counters: vec![0; KERNEL_KEYS as usize],
        }
    }

    fn genesis_writes(&self) -> (Writes, Writes) {
        let mut user = Vec::with_capacity(ACCOUNTS as usize * 3 + 64);
        for i in 0..ACCOUNTS {
            user.push((
                balance_key(i),
                Some(balance_value(self.balances[i as usize])),
            ));
            user.push((nonce_key(i), Some(nonce_value(self.nonces[i as usize]))));
            user.push((account_key(i), Some(account_value(i))));
        }
        for i in 0..HOT_READ_ONLY_KEYS {
            user.push((hot_read_only_key(i), Some(fixed_value(b"hot", i))));
        }
        for i in 0..HOT_READ_WRITE_KEYS {
            user.push((
                hot_read_write_key(i),
                Some(balance_value(self.fees[i as usize])),
            ));
        }
        let kernel = (0..KERNEL_KEYS)
            .map(|i| {
                (
                    kernel_key(i),
                    Some(nonce_value(self.kernel_counters[i as usize])),
                )
            })
            .collect();
        (user, kernel)
    }

    /// Builds the accesses of one slot and applies its writes to the model.
    fn slot_accesses(&mut self, slot: u64) -> StateAccesses {
        let mut rng = SplitMix64(SEED ^ slot.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let existing = self.balances.len() as u32;
        let mut used = BTreeSet::new();
        let mut pick = |rng: &mut SplitMix64| loop {
            let candidate = rng.below(existing);
            if used.insert(candidate) {
                return candidate;
            }
        };

        let mut reads = Vec::new();
        let mut writes = Vec::new();
        for t in 0..TRANSFERS_PER_SLOT {
            let sender = pick(&mut rng);
            let amount = u128::from(rng.below(1_000) + 1);

            let sender_balance = balance_value(self.balances[sender as usize]);
            let sender_nonce = nonce_value(self.nonces[sender as usize]);
            reads.push((balance_key(sender), leaf(&sender_balance)));
            reads.push((nonce_key(sender), leaf(&sender_nonce)));
            reads.push((account_key(sender), leaf(&account_value(sender))));
            self.balances[sender as usize] -= amount;
            self.nonces[sender as usize] += 1;
            writes.push((
                balance_key(sender),
                Some(balance_value(self.balances[sender as usize])),
            ));
            writes.push((
                nonce_key(sender),
                Some(nonce_value(self.nonces[sender as usize])),
            ));

            let receiver = if t % NEW_RECEIVER_EVERY == 0 {
                self.balances.push(0);
                self.nonces.push(0);
                reads.push((balance_key(self.balances.len() as u32 - 1), None));
                self.balances.len() as u32 - 1
            } else {
                let receiver = pick(&mut rng);
                let receiver_balance = balance_value(self.balances[receiver as usize]);
                reads.push((balance_key(receiver), leaf(&receiver_balance)));
                if t % RECEIVER_ACCOUNT_READ_EVERY == 1 {
                    reads.push((account_key(receiver), leaf(&account_value(receiver))));
                }
                receiver
            };
            self.balances[receiver as usize] += amount;
            writes.push((
                balance_key(receiver),
                Some(balance_value(self.balances[receiver as usize])),
            ));
        }

        for i in 0..HOT_READ_ONLY_KEYS {
            reads.push((hot_read_only_key(i), leaf(&fixed_value(b"hot", i))));
        }
        for i in 0..HOT_READ_WRITE_KEYS {
            let fees = &mut self.fees[i as usize];
            reads.push((hot_read_write_key(i), leaf(&balance_value(*fees))));
            *fees += u128::from(TRANSFERS_PER_SLOT);
            writes.push((hot_read_write_key(i), Some(balance_value(*fees))));
        }

        let mut kernel_reads = Vec::new();
        let mut kernel_writes = Vec::new();
        for i in 0..KERNEL_READ_WRITE_PER_SLOT {
            let counter = &mut self.kernel_counters[i as usize];
            kernel_reads.push((kernel_key(i), leaf(&nonce_value(*counter))));
            *counter += 1;
            kernel_writes.push((kernel_key(i), Some(nonce_value(*counter))));
        }
        let mut kernel_used = BTreeSet::new();
        while kernel_used.len() < KERNEL_READ_ONLY_PER_SLOT as usize {
            let i =
                KERNEL_READ_WRITE_PER_SLOT + rng.below(KERNEL_KEYS - KERNEL_READ_WRITE_PER_SLOT);
            if kernel_used.insert(i) {
                kernel_reads.push((
                    kernel_key(i),
                    leaf(&nonce_value(self.kernel_counters[i as usize])),
                ));
            }
        }

        StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: reads,
                ordered_writes: writes,
            },
            kernel: OrderedReadsAndWrites {
                ordered_reads: kernel_reads,
                ordered_writes: kernel_writes,
            },
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn storage_config(path: &Path) -> RollupDbConfig {
    let config_path = workspace_root().join("examples/demo-rollup/configs/mock_rollup_config.toml");
    let raw = std::fs::read_to_string(&config_path).expect("read mock_rollup_config.toml");
    let table: toml::Table = raw.parse().expect("parse mock_rollup_config.toml");
    let mut config: RollupDbConfig = table["storage"]
        .clone()
        .try_into()
        .expect("deserialize [storage] section");
    config.path = path.to_path_buf();
    config
}

struct Session {
    manager: BenchStorageManager,
    height: u64,
    root: Root,
}

impl Session {
    fn open(path: &Path) -> Self {
        let manager = BenchStorageManager::new(storage_config(path), false).expect("open storage");
        let (height, root) = match std::fs::read(path.join(TIP_FILE)) {
            Ok(bytes) => borsh::from_slice(&bytes).expect("decode tip"),
            Err(_) => (0, BenchStorage::PRE_GENESIS_ROOT),
        };
        Self {
            manager,
            height,
            root,
        }
    }

    fn save_tip(&self, path: &Path) {
        std::fs::write(
            path.join(TIP_FILE),
            borsh::to_vec(&(self.height, self.root)).unwrap(),
        )
        .unwrap();
    }

    /// Applies one slot; returns the time spent in `compute_state_update`.
    fn apply(&mut self, accesses: StateAccesses) -> Duration {
        self.height += 1;
        let block = MockBlock::default_at_height(self.height);
        let header = block.header().clone();
        let (storage, ledger_reader) = self.manager.create_state_for(&header).unwrap();
        let start = Instant::now();
        let (root, update) = storage
            .compute_state_update(accesses, &Default::default(), self.root)
            .expect("compute_state_update");
        let elapsed = start.elapsed();
        // The storage manager checks the ledger head root on startup, so record a minimal slot.
        let ledger_changes = LedgerDb::with_reader(ledger_reader)
            .unwrap()
            .materialize_slot(
                SlotCommit::<_, (), ()>::new(block, Vec::new()),
                root.as_ref(),
            )
            .unwrap();
        self.manager
            .save_change_set(&header, storage.materialize_changes(update), ledger_changes)
            .unwrap();
        self.manager.finalize(&header).unwrap();
        self.root = root;
        elapsed
    }
}

fn generate_snapshot(snapshot: &Path) {
    let _ = std::fs::remove_dir_all(snapshot);
    std::fs::create_dir_all(snapshot).unwrap();
    let start = Instant::now();
    let mut session = Session::open(snapshot);
    let (mut user, mut kernel) = Model::genesis().genesis_writes();
    while !user.is_empty() || !kernel.is_empty() {
        let user_batch: Vec<_> = user.drain(..GENESIS_BATCH.min(user.len())).collect();
        // Every slot must touch the kernel namespace; later batches rewrite key 0 with its genesis value.
        let mut kernel_batch: Vec<_> = std::mem::take(&mut kernel);
        if kernel_batch.is_empty() {
            kernel_batch.push((kernel_key(0), Some(nonce_value(0))));
        }
        session.apply(StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: user_batch,
            },
            kernel: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: kernel_batch,
            },
        });
    }
    session.save_tip(snapshot);
    drop(session);
    std::fs::write(snapshot.join("READY"), SNAPSHOT_VERSION.to_string()).unwrap();
    eprintln!("generated snapshot in {:?}", start.elapsed());
}

fn millis(d: Duration) -> String {
    let micros = d.as_micros();
    format!("{}.{:03}", micros / 1000, micros % 1000)
}

fn main() {
    let run_start = Instant::now();
    let root = std::env::var_os("FINALIZATION_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target/finalization-bench"));
    let snapshot = root.join(format!("snapshot-v{SNAPSHOT_VERSION}"));
    if std::fs::read_to_string(snapshot.join("READY"))
        .ok()
        .as_deref()
        != Some(SNAPSHOT_VERSION.to_string().as_str())
    {
        generate_snapshot(&snapshot);
    }

    let work = root.join("work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    // One `cp` per database directory, in parallel.
    let copies: Vec<_> = std::fs::read_dir(&snapshot)
        .unwrap()
        .map(|entry| {
            Command::new("cp")
                .arg("-a")
                .arg("--sparse=always")
                .arg(entry.unwrap().path())
                .arg(&work)
                .spawn()
                .expect("spawn cp")
        })
        .collect();
    for mut copy in copies {
        assert!(
            copy.wait().unwrap().success(),
            "copying the snapshot failed"
        );
    }

    let mut session = Session::open(&work);
    let mut model = Model::genesis();
    // Genesis batches are not part of the transfer load; slots are numbered after them.
    let first_slot = session.height + 1;

    let mut times = Vec::new();
    let (mut user_reads, mut user_writes) = (0, 0);
    for slot in 0..WARMUP_SLOTS + TIMED_SLOTS {
        let accesses = model.slot_accesses(first_slot + slot);
        user_reads = accesses.user.ordered_reads.len();
        user_writes = accesses.user.ordered_writes.len();
        let elapsed = session.apply(accesses);
        if slot >= WARMUP_SLOTS {
            times.push(elapsed);
        }
    }
    drop(session);
    let _ = std::fs::remove_dir_all(&work);

    times.sort();
    let p90 = times[(times.len() * 9).div_ceil(10) - 1];
    println!("user_reads_per_slot: {user_reads}");
    println!("user_writes_per_slot: {user_writes}");
    println!("finalization_ms_min: {}", millis(times[0]));
    println!("finalization_ms_median: {}", millis(times[times.len() / 2]));
    println!("finalization_ms_p90: {}", millis(p90));
    println!("finalization_ms_max: {}", millis(times[times.len() - 1]));
    println!("run_time_s: {}", run_start.elapsed().as_secs());
}
