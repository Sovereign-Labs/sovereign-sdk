use itertools::Itertools;
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
        for snapshot in self.snapshots[..snapshot_id as usize].iter().rev() {
            let iter = snapshot.iter::<S>();
            iterators.push(iter);
        }

        let merged = itertools::kmerge_by(
            iterators,
            |&(key_a, _): &(&SchemaKey, &Operation), &(key_b, _): &(&SchemaKey, &Operation)| {
                key_a < key_b
            },
        );

        // let numbers: Vec<Sample> = merged.collect();
        let result = merged
            .group_by(|&(key, _)| key)
            .into_iter()
            .filter_map(|(_, group)| match group.last() {
                None => None,
                Some((_, Operation::Delete)) => None,
                Some((key, Operation::Put { value })) => Some((key.to_vec(), value.to_vec())),
            })
            .collect::<Vec<_>>();

        Ok(result.into_iter())
    }
}
