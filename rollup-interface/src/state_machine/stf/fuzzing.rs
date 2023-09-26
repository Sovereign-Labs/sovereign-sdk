//! Implements fuzzing strategies for structs in the stf module

use digest::typenum::U32;
use digest::Digest;
use proptest::prelude::{any, Arbitrary};
use proptest::strategy::{BoxedStrategy, Strategy};

use super::{BatchReceipt, Event, TransactionReceipt};

/// An object-safe hashing trait, which is blanket implemented for all
/// [`digest::Digest`] implementors.
pub trait FuzzHasher {
    /// Hash the provided data
    fn hash(&self, data: &[u8]) -> [u8; 32];
}

/// The default hasher to use for fuzzing
fn default_fuzz_hasher() -> Box<dyn FuzzHasher> {
    Box::new(sha2::Sha256::new())
}

impl<T: Digest<OutputSize = U32> + Clone> FuzzHasher for T {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = T::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

/// A special merkle hasher used only for fuzz tests. This hasher sacrifices some
/// efficiency for object safety.
struct FuzzMerkleHasher<'a> {
    #[allow(clippy::borrowed_box)]
    hasher: &'a Box<dyn FuzzHasher>,
}

impl<'a> FuzzMerkleHasher<'a> {
    fn empty_root(&mut self) -> [u8; 32] {
        self.hasher.hash(&[])
    }

    fn leaf_hash(&mut self, bytes: impl AsRef<[u8]>) -> [u8; 32] {
        let bytes = bytes.as_ref();
        let mut input = Vec::with_capacity(1 + bytes.len());
        input.push(0);
        input.extend_from_slice(bytes);
        self.hasher.hash(input.as_ref())
    }

    fn inner_hash(&mut self, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut input = Vec::with_capacity(32 + 32 + 1);
        input.push(1);
        input.extend_from_slice(left);
        input.extend_from_slice(right);
        self.hasher.hash(&input)
    }

    /// A trait describing how to build a merkle tree from a slice of bytes. The default implementation
    /// returns an RFC6962-style tree for ease of updating.
    pub fn build_merkle_tree(&mut self, byte_vecs: &[impl AsRef<[u8]>]) -> [u8; 32] {
        let length = byte_vecs.len();
        match length {
            0 => self.empty_root(),
            1 => self.leaf_hash(byte_vecs[0].as_ref()),
            _ => {
                let split = length.next_power_of_two() / 2;
                let left = self.build_merkle_tree(&byte_vecs[..split]);
                let right = self.build_merkle_tree(&byte_vecs[split..]);
                self.inner_hash(&left, &right)
            }
        }
    }
}

/// How frequently to generate a particular variant of an enum.
pub enum Frequency {
    /// The variant will always be generated
    Always,
    /// The variant will sometimes be generated
    Sometimes,
    /// The variant will never be generated
    Never,
}

/// The arguments needed to construct a transaction receipt generation strategy
pub struct TransactionReceiptStrategyArgs {
    /// An optional SimpleHasher to use construct the tx hash from the `body_to_save`.
    /// If no hasher is provided or the tx `body_to_save` is `None`, the hash will be populated
    /// with random bytes
    pub hasher: Option<Box<dyn FuzzHasher>>,
    /// The maximum number of events to generate for this transaction. Defaults to 10
    pub max_events: usize,
    /// Whether to generate entries for the `body_to_save` field of the `TransactionReceipt`.
    /// Values are guaranteed to be `Some` if this is `Always`, and `None` if this is `Never`.
    pub generate_tx_bodies: Frequency,
}

impl Default for TransactionReceiptStrategyArgs {
    fn default() -> Self {
        Self {
            hasher: Some(default_fuzz_hasher()),
            max_events: 10,
            generate_tx_bodies: Frequency::Sometimes,
        }
    }
}

impl<R: proptest::arbitrary::Arbitrary + 'static> proptest::arbitrary::Arbitrary
    for TransactionReceipt<R>
{
    type Parameters = TransactionReceiptStrategyArgs;
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        {
            let tx_body_strategy = match args.generate_tx_bodies {
                Frequency::Always => any::<Vec<u8>>().prop_map(Some).boxed(),
                Frequency::Sometimes => any::<Option<Vec<u8>>>().boxed(),
                Frequency::Never => proptest::collection::vec(any::<u8>(), 0..0)
                    .prop_map(|_| None)
                    .boxed(),
            };
            (
                any::<[u8; 32]>(),
                tx_body_strategy,
                proptest::collection::vec(any::<Event>(), 0..args.max_events),
                any::<R>(),
            )
                .prop_map(move |(tx_hash, body_to_save, events, receipt)| {
                    let tx_hash = match (args.hasher.as_ref(), body_to_save.as_ref()) {
                        (Some(hasher), Some(body)) => hasher.hash(body),
                        _ => tx_hash,
                    };
                    Self {
                        tx_hash,
                        body_to_save,
                        events,
                        receipt,
                    }
                })
                .boxed()
        }
    }
}

/// The arguments needed to construct a batch receipt generation strategy.
pub struct BatchReceiptStrategyArgs {
    /// An optional [`digest::Digest`] to use construct the tx hash from the `body_to_save`.
    /// If no hasher is provided or the tx `body_to_save` is `None`, the hash will be populated
    /// with random bytes
    pub hasher: Option<Box<dyn FuzzHasher>>,
    /// The maximum number of events to generate for this transaction. Defaults to 10
    pub max_txs: usize,
    /// The arguments to use for generating transactions in this batch
    pub transaction_strategy_args: TransactionReceiptStrategyArgs,
}

impl Default for BatchReceiptStrategyArgs {
    fn default() -> Self {
        Self {
            hasher: Some(default_fuzz_hasher()),
            max_txs: 10,
            transaction_strategy_args: Default::default(),
        }
    }
}

impl<B: Arbitrary + 'static, R: Arbitrary + 'static> Arbitrary for BatchReceipt<B, R> {
    type Parameters = BatchReceiptStrategyArgs;
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        {
            (
                any::<[u8; 32]>(),
                proptest::collection::vec(
                    TransactionReceipt::arbitrary_with(args.transaction_strategy_args),
                    0..args.max_txs,
                ),
                any::<B>(),
            )
                .prop_map(move |(batch_hash, txs, receipt)| {
                    let batch_hash = match args.hasher {
                        Some(ref hasher) => {
                            let mut merkle_hasher = FuzzMerkleHasher { hasher };
                            let tx_hashes = txs.iter().map(|tx| &tx.tx_hash).collect::<Vec<_>>();
                            merkle_hasher.build_merkle_tree(&tx_hashes)
                        }
                        None => batch_hash,
                    };
                    Self {
                        batch_hash,
                        tx_receipts: txs,
                        inner: receipt,
                    }
                })
                .boxed()
        }
    }
}
