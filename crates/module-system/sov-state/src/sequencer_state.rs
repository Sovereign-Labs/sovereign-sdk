use std::{collections::{HashMap, VecDeque}, sync::Arc};


#[cfg(feature = "native")]
use crate::ProvableNamespace;
use crate::{namespaces, AccessoryWrite, Namespace,  ProvableStorageCache, SlotKey, SlotValue, StateGetter};

/// The list of state changes for a single rollup block.
#[derive(Debug, Clone)]
pub struct RawStateChanges {
	user: ProvableStorageCache<namespaces::User>,
    kernel: ProvableStorageCache<namespaces::Kernel>,
    accessory: HashMap<SlotKey, AccessoryWrite>,
	/// The rollup height at which the changes were made.
	pub rollup_height: u64,
}


/// A collection of state changes from contiguous blocks, ordered by the rollup height at which they were made - newest first.
#[derive(Debug, Clone)]
pub struct SequencerStateChanges {
	/// The changes, ordered by the rollup height at which they were made
	pub changes: Option<VecDeque<Arc<RawStateChanges>>>,
}

 
pub enum MaybePresentValue<T = SlotValue> {
	Present(Option<T>),
	Absent,
}

impl<T> MaybePresentValue<T> {
	pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> MaybePresentValue<U> {
		match self {
			MaybePresentValue::Present(value) => MaybePresentValue::Present(value.map(f)),
			MaybePresentValue::Absent => MaybePresentValue::Absent,
		}
	}

	pub fn or_else<F: FnOnce() -> Option<T>>(self, f: F) -> Option<T> {
		match self {
			MaybePresentValue::Present(value) => value,
			MaybePresentValue::Absent => f(),
		}
	}
}

#[cfg(feature = "native")]
impl StateGetter for SequencerStateChanges {
	fn get(&self, namespace: Namespace, key: &SlotKey) -> MaybePresentValue {
		for change_set in self.changes.iter().flatten() {
			if let MaybePresentValue::Present(maybe_value) = match namespace {
					Namespace::User => change_set.user.get_from_cache(key),
						Namespace::Kernel => change_set.kernel.get_from_cache(key),
						Namespace::Accessory => match change_set.accessory.get(key).map(|write| write.value.clone()) {
							Some(maybe_value) => MaybePresentValue::Present(maybe_value),
							None => MaybePresentValue::Absent,
						},
				}  {
					return MaybePresentValue::Present(maybe_value);
				}
			}
		MaybePresentValue::Absent
	}

	fn get_leaf(
			&self,
			namespace: ProvableNamespace,
			key: &SlotKey,
		) -> MaybePresentValue<crate::NodeLeafAndMaybeValue> {
			for change_set in self.changes.iter().flatten() {
				if let MaybePresentValue::Present(maybe_value) = match namespace {
					ProvableNamespace::User => change_set.user.get_leaf_from_cache_with_dummy_hash(key),
					ProvableNamespace::Kernel => change_set.kernel.get_leaf_from_cache_with_dummy_hash(key),
				} {
					return MaybePresentValue::Present(maybe_value);
				}
			}
			MaybePresentValue::Absent
		}
	
	fn box_clone(&self) -> Box<dyn StateGetter> {
		Box::new(self.clone())
	}
}

// The problem with implementing this on `Storage` is that its pointlessly expensive; we may have to hash the 
// value. 
// 
// Alternatively, we could have the `SequencerStateChanges` live in-between the Delta and the Storage. Then...
// - We don't ever need to do the `get_leaf` thing (only `get_size`, which is efficient)
// - For optimistic execution what do we need? 
// - For each tx, we need...
//    - A list of the values it read (in order, even if already in cache)
//    - A list of the values that it wrote. 
// - One idea: we give each tx a fresh StateCheckpoint (empty cache) and throw any *previous* checkpoints that are finalized in this
// in-between layer as soon as they're ready. Then...
//   The first checkpoint works as expected. 
//   The second tx assembles a complete list of reads/writes in its checkpoint. On commit we...
//     - Wait for the previous tx to commit.
//     - For each read, check that the value at that key either... was not changed by the previous tx or was changed to the read value..
//     - If all checks pass, commit the tx into the "previous" storage. Otherwise, re-execute the tx.
// 
// For this to work well we need to ensure that...
//   - The list of completed checkpoints we're working from is immutable for the lifetime of the block. (We should make each previous one an Arc)
//   - Completed checkpoints are cheap to clone
