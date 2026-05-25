#![no_main]

sp1_zkvm::entrypoint!(main);

use core::hint::black_box;

use nomt_core::hasher::{BinaryHasher, NodeHasher};
use nomt_core::proof::{
    verify_multi_proof, verify_multi_proof_update, MultiProof, PathProof, PathProofTerminal,
};
use nomt_core::trie::{InternalData, KeyPath, LeafData, Node, ValueHash};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::{CryptoSpec, Spec};

type MicrobenchSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Zk>;
// Same hasher the production NOMT verifier uses (`BinaryHasher<S::Hasher>`), so the swept
// per-depth cost is the real, SP1-accelerated sha256 the prover pays.
type Hasher = BinaryHasher<<<MicrobenchSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher>;

// Mode flags must match cmd/storage.rs.
const MODE_READ: u8 = 0;
const MODE_WRITE: u8 = 1;

pub fn main() {
    let mode: u8 = sp1_zkvm::io::read();
    let depth: u32 = sp1_zkvm::io::read();
    let iterations: u32 = sp1_zkvm::io::read();

    let (proof, root, key_path) = build_single_path_proof(depth as usize);

    match mode {
        MODE_READ => run_read(&proof, root, iterations),
        MODE_WRITE => run_write(&proof, root, key_path, iterations),
        other => panic!("unknown storage bench mode {other}"),
    }
}

/// Verifies an inclusion proof for one key at the given trie depth — the per-access cost charged
/// via `bias_to_charge_for_access`.
fn run_read(proof: &MultiProof, root: Node, iterations: u32) {
    println!("cycle-tracker-report-start: storage_loop");
    for _ in 0..iterations {
        let verified = verify_multi_proof::<Hasher>(black_box(proof), black_box(root))
            .expect("hand-built proof must verify");
        black_box(&verified);
    }
    println!("cycle-tracker-report-end: storage_loop");

    sp1_zkvm::io::commit(&root);
}

/// Verifies a value update for one key — the extra cost a write pays via `BIAS_STORAGE_UPDATE`,
/// on top of the access cost measured by [`run_read`].
fn run_write(proof: &MultiProof, root: Node, key_path: KeyPath, iterations: u32) {
    let verified =
        verify_multi_proof::<Hasher>(proof, root).expect("hand-built proof must verify");
    let new_value: ValueHash = fill(0x03);

    println!("cycle-tracker-report-start: storage_loop");
    let mut last = root;
    for _ in 0..iterations {
        let ops = vec![(key_path, Some(black_box(new_value)))];
        last = verify_multi_proof_update::<Hasher>(black_box(&verified), black_box(ops))
            .expect("hand-built update must verify");
        black_box(&last);
    }
    println!("cycle-tracker-report-end: storage_loop");

    sp1_zkvm::io::commit(&last);
}

/// Builds a valid single-key inclusion proof of exactly `depth` siblings, plus the root it proves
/// against. Construction is iteration-independent, so the two-iteration differencing cancels it.
fn build_single_path_proof(depth: usize) -> (MultiProof, Node, KeyPath) {
    let key_path: KeyPath = fill(0x01);
    let value_hash: ValueHash = fill(0x02);
    let leaf = LeafData {
        key_path,
        value_hash,
    };
    let siblings: Vec<Node> = (0..depth).map(|d| fill(0x40u8.wrapping_add(d as u8))).collect();

    let root = compute_root::<Hasher>(&leaf, &siblings);

    let proof = MultiProof::from_path_proofs(vec![PathProof {
        terminal: PathProofTerminal::Leaf(leaf),
        siblings,
    }]);
    (proof, root, key_path)
}

/// Folds the leaf up through its siblings to the root, mirroring nomt-core's (non-exported)
/// `hash_path`: siblings are ascending by depth and consumed from the bottom up.
fn compute_root<H: NodeHasher>(leaf: &LeafData, siblings: &[Node]) -> Node {
    let mut node = H::hash_leaf(leaf);
    for depth in (0..siblings.len()).rev() {
        let sibling = siblings[depth];
        let internal = if bit_at(&leaf.key_path, depth) {
            InternalData {
                left: sibling,
                right: node,
            }
        } else {
            InternalData {
                left: node,
                right: sibling,
            }
        };
        node = H::hash_internal(&internal);
    }
    node
}

/// Bit at index `i` of a key path, most-significant-bit first (matching nomt's `Msb0` paths).
fn bit_at(key: &KeyPath, i: usize) -> bool {
    (key[i / 8] >> (7 - (i % 8))) & 1 == 1
}

fn fill(seed: u8) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = seed.wrapping_add(i as u8).wrapping_mul(0xAB);
    }
    bytes
}
