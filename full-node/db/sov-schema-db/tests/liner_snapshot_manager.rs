use std::cmp::Ordering;

use sov_schema_db::schema::{KeyCodec, ValueCodec};
use sov_schema_db::snapshot::{FrozenDbSnapshot, QueryManager, SnapshotId};
use sov_schema_db::{Operation, Schema, SchemaKey, SchemaValue};

#[derive(Default)]
pub struct LinearSnapshotManager {
    snapshots: Vec<FrozenDbSnapshot>,
}

impl LinearSnapshotManager {
    #[allow(dead_code)]
    pub fn add_snapshot(&mut self, snapshot: FrozenDbSnapshot) {
        self.snapshots.push(snapshot);
    }
}

impl QueryManager for LinearSnapshotManager {
    type Iter<'a, S: Schema> = std::vec::IntoIter<(SchemaKey, SchemaValue)>;

    fn get<S: Schema>(
        &self,
        snapshot_id: SnapshotId,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<S::Value>> {
        for snapshot in self.snapshots[..snapshot_id as usize].iter().rev() {
            if let Some(operation) = snapshot.get(key)? {
                return match operation {
                    Operation::Put { value } => Ok(Some(S::Value::decode_value(value)?)),
                    Operation::Delete => Ok(None),
                };
            }
        }
        Ok(None)
    }

    // For simplicity it just stores all values in the vector that is returned
    fn iter<S: Schema>(&self, snapshot_id: SnapshotId) -> anyhow::Result<Self::Iter<'_, S>> {
        let mut iterators = vec![];
        // The most recent snapshot is on the right(end of the vector)
        for snapshot in self.snapshots[..snapshot_id as usize].iter() {
            let iter = snapshot.iter::<S>();
            iterators.push(iter.peekable());
        }

        let mut result = vec![];
        loop {
            // We need several equal max values together,
            // so snapshots with same keys, can progress together
            let mut max_values: Vec<(usize, &SchemaKey)> = vec![];
            for (idx, iter) in iterators.iter_mut().enumerate() {
                if let Some(&(peeked_key, _)) = iter.peek() {
                    if max_values.is_empty() {
                        max_values.push((idx, peeked_key));
                    } else {
                        let (_, max_key) = &max_values[0];
                        match peeked_key.cmp(max_key) {
                            Ordering::Greater => {
                                max_values.clear();
                                max_values.push((idx, peeked_key));
                            }
                            Ordering::Equal => {
                                max_values.push((idx, peeked_key));
                            }
                            Ordering::Less => {}
                        }
                    }
                }
            }

            if max_values.is_empty() {
                break;
            }
            // Dropping &mut to iterators, as we got all indexes we need
            let mut max_values: Vec<usize> = max_values.into_iter().map(|(idx, _)| idx).collect();

            // We return most recent value for the key, which from the latest snapshot
            let last_max_idx = max_values.pop().unwrap();

            // Progressing all other iterators
            for idx in max_values {
                iterators[idx].next();
            }

            // If the most recent operation is delete, moving further
            let (next_k, next_op) = iterators[last_max_idx].next().unwrap();
            if let Operation::Put { value } = next_op {
                result.push((next_k.to_vec(), value.to_vec()))
            }
        }

        Ok(result.into_iter())
    }
}
