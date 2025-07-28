use std::sync::Arc;

use rockbound::schema::{KeyCodec, Schema};
use rockbound::{SchemaBatch, SchemaIterator, SchemaKey, SeekKeyEncoder, DB};
use sov_rollup_interface::common::SlotNumber;

use crate::metrics::nomt::PrunerMetric;
use crate::schema::tables::ModuleAccessoryState;

type VersionedSchemaKey = (SchemaKey, SlotNumber);

/// Allows pruning old versions of keys.
pub struct Pruner {
    db: Arc<DB>,
}

impl Pruner {
    /// Creates a new pruner instance with the given database.
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Gathers all delete operations that can prune older versions.
    pub fn collect_pruning_batch<T>(&self, keep_versions: u64) -> anyhow::Result<SchemaBatch>
    where
        T: Schema<Key = VersionedSchemaKey>,
        VersionedSchemaKey: SeekKeyEncoder<T> + KeyCodec<T>,
    {
        let start = std::time::Instant::now();
        let mut keys_inspected = 0;
        let mut keys_to_prune = 0;
        let mut pruning_batch = SchemaBatch::new();
        if keep_versions == 0 {
            return Err(anyhow::anyhow!("keep_versions must be at least 1"));
        }

        let unique_base_keys = UniqueBaseKeysIterator::new(&self.db)?;

        // For each unique base key, find versions to prune
        for base_key in unique_base_keys {
            keys_inspected += 1;
            let base_key = base_key?;
            if let Some((oldest_version, last_prunable_version)) =
                self.get_prunable_version_for_base_key::<T>(&base_key, keep_versions)?
            {
                keys_to_prune += 1;
                pruning_batch.delete_range::<T>(
                    &(base_key.clone(), oldest_version),
                    &(base_key, last_prunable_version),
                )?;
            }
        }
        let pruning_time = start.elapsed();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(PrunerMetric {
                db: self.db.name(),
                keys_inspected,
                keys_to_prune,
                time: pruning_time,
            });
        });

        Ok(pruning_batch)
    }

    /// Can be pruned up to a returned version (included).
    /// Create an iterator for a given key between version genesis and max.
    /// Seek to last and skip keep versions in a direction of the start.
    /// If there's something, meaning it can be pruned.
    fn get_prunable_version_for_base_key<T>(
        &self,
        base_key: &SchemaKey,
        keep_versions: u64,
    ) -> anyhow::Result<Option<(SlotNumber, SlotNumber)>>
    where
        T: Schema<Key = VersionedSchemaKey>,
        VersionedSchemaKey: SeekKeyEncoder<T>,
    {
        let start = (base_key.clone(), SlotNumber::GENESIS);
        let end = (base_key.clone(), SlotNumber::MAX);
        let mut iter = self.db.iter_range::<T>(&start, &end)?.rev();
        iter.seek_to_last();

        let mut iter = iter.skip(keep_versions as usize);

        // Making range inclusive
        let last_version_to_prune = iter.next().transpose()?.map(|n| n.key.1.saturating_add(1));
        let range = match last_version_to_prune {
            None => None,
            Some(end_range) => {
                // Not ideal, as a second iterator, but more effective in non-pruning case.
                let mut iter = self.db.iter_range::<T>(&start, &end)?.rev();
                iter.seek_to_first();
                let start = iter
                    .next()
                    .transpose()?
                    .expect("Start range should be found with non-empty end range");

                Some((start.key.1, end_range))
            }
        };
        Ok(range)
    }

    /// For accessory DB
    pub fn collect_pruning_batch_for_module_accessory_state(
        &self,
        keep_versions: u64,
    ) -> anyhow::Result<SchemaBatch> {
        self.collect_pruning_batch::<ModuleAccessoryState>(keep_versions)
    }
}

struct UniqueBaseKeysIterator<'a, T>
where
    T: Schema<Key = VersionedSchemaKey> + 'a,
    VersionedSchemaKey: SeekKeyEncoder<T>,
{
    iter: SchemaIterator<'a, T>,
}

impl<'a, T> UniqueBaseKeysIterator<'a, T>
where
    T: Schema<Key = VersionedSchemaKey> + 'a,
    VersionedSchemaKey: SeekKeyEncoder<T>,
{
    fn new(db: &'a DB) -> anyhow::Result<Self> {
        let mut iter = db.iter::<T>()?.rev();
        iter.seek_to_last();
        Ok(Self { iter })
    }
}

impl<'a, T> Iterator for UniqueBaseKeysIterator<'a, T>
where
    T: Schema<Key = VersionedSchemaKey> + 'a,
    VersionedSchemaKey: SeekKeyEncoder<T>,
{
    type Item = anyhow::Result<SchemaKey>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next()? {
            Ok(n) => {
                let (base, _version) = n.key;
                let zero = (base.clone(), SlotNumber::GENESIS);
                if let Err(e) = self.iter.seek(&zero) {
                    return Some(Err(e));
                }
                self.iter.next();
                Some(Ok(base))
            }
            Err(err) => Some(Err(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sov_rollup_interface::common::SlotNumber;

    use crate::accessory_db::AccessoryDb;
    use crate::pruner::Pruner;
    use crate::schema::tables::ModuleAccessoryState;

    #[test]
    fn test_pruner() {
        // Here is data, where the first column is a key,
        // other columns are versions, cells are values:
        //
        // | _ | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
        // | A | _ | _ | _ | _ | _ | _ | 0 | 1 | 2 | 3 |
        // | B | _ | _ | _ | _ | _ | _ | _ | 0 | 1 | 2 |
        // | C | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
        // | D | 0 | _ | _ | 1 | _ | _ | 2 | _ | _ | 3 |
        // | E | 0 | 1 | 2 | _ | _ | _ | _ | _ | _ | _ |
        // | F | 0 | 1 | 2 | 3 | _ | _ | _ | _ | _ | _ |
        // | G | 0 | _ | _ | _ | _ | _ | _ | 1 | 2 | 3 |
        //
        // We set to keep only 3 versions.

        let tempdir = tempfile::tempdir().unwrap();
        let rocksdb = Arc::new(
            AccessoryDb::get_rockbound_options()
                .default_setup_db_in_path(tempdir.path())
                .unwrap(),
        );

        // Version 0: C=0, D=0, E=0, F=0, G=0
        let data_0 = AccessoryDb::materialize_values(
            vec![
                (b"C".to_vec(), Some(vec![0])),
                (b"D".to_vec(), Some(vec![0])),
                (b"E".to_vec(), Some(vec![0])),
                (b"F".to_vec(), Some(vec![0])),
                (b"G".to_vec(), Some(vec![0])),
            ],
            SlotNumber::new(0),
        )
        .unwrap();
        rocksdb.write_schemas(&data_0).unwrap();

        // Version 1: C=1, E=1, F=1
        let data_1 = AccessoryDb::materialize_values(
            vec![
                (b"C".to_vec(), Some(vec![1])),
                (b"E".to_vec(), Some(vec![1])),
                (b"F".to_vec(), Some(vec![1])),
            ],
            SlotNumber::new(1),
        )
        .unwrap();
        rocksdb.write_schemas(&data_1).unwrap();

        // Version 2: C=2, E=2, F=2
        let data_2 = AccessoryDb::materialize_values(
            vec![
                (b"C".to_vec(), Some(vec![2])),
                (b"E".to_vec(), Some(vec![2])),
                (b"F".to_vec(), Some(vec![2])),
            ],
            SlotNumber::new(2),
        )
        .unwrap();
        rocksdb.write_schemas(&data_2).unwrap();

        // Version 3: C=3, D=1, F=3
        let data_3 = AccessoryDb::materialize_values(
            vec![
                (b"C".to_vec(), Some(vec![3])),
                (b"D".to_vec(), Some(vec![1])),
                (b"F".to_vec(), Some(vec![3])),
            ],
            SlotNumber::new(3),
        )
        .unwrap();
        rocksdb.write_schemas(&data_3).unwrap();

        // Version 4: C=4
        let data_4 = AccessoryDb::materialize_values(
            vec![(b"C".to_vec(), Some(vec![4]))],
            SlotNumber::new(4),
        )
        .unwrap();
        rocksdb.write_schemas(&data_4).unwrap();

        // Version 5: C=5
        let data_5 = AccessoryDb::materialize_values(
            vec![(b"C".to_vec(), Some(vec![5]))],
            SlotNumber::new(5),
        )
        .unwrap();
        rocksdb.write_schemas(&data_5).unwrap();

        // Version 6: A=0, C=6, D=2
        let data_6 = AccessoryDb::materialize_values(
            vec![
                (b"A".to_vec(), Some(vec![0])),
                (b"C".to_vec(), Some(vec![6])),
                (b"D".to_vec(), Some(vec![2])),
            ],
            SlotNumber::new(6),
        )
        .unwrap();
        rocksdb.write_schemas(&data_6).unwrap();

        // Version 7: A=1, B=0, C=7, G=1
        let data_7 = AccessoryDb::materialize_values(
            vec![
                (b"A".to_vec(), Some(vec![1])),
                (b"B".to_vec(), Some(vec![0])),
                (b"C".to_vec(), Some(vec![7])),
                (b"G".to_vec(), Some(vec![1])),
            ],
            SlotNumber::new(7),
        )
        .unwrap();
        rocksdb.write_schemas(&data_7).unwrap();

        // Version 8: A=2, B=1, C=8, G=2
        let data_8 = AccessoryDb::materialize_values(
            vec![
                (b"A".to_vec(), Some(vec![2])),
                (b"B".to_vec(), Some(vec![1])),
                (b"C".to_vec(), Some(vec![8])),
                (b"G".to_vec(), Some(vec![2])),
            ],
            SlotNumber::new(8),
        )
        .unwrap();
        rocksdb.write_schemas(&data_8).unwrap();

        // Version 9: A=3, B=2, C=9, D=3, G=3
        let data_9 = AccessoryDb::materialize_values(
            vec![
                (b"A".to_vec(), Some(vec![3])),
                (b"B".to_vec(), Some(vec![2])),
                (b"C".to_vec(), Some(vec![9])),
                (b"D".to_vec(), Some(vec![3])),
                (b"G".to_vec(), Some(vec![3])),
            ],
            SlotNumber::new(9),
        )
        .unwrap();
        rocksdb.write_schemas(&data_9).unwrap();

        let pruner = Pruner::new(rocksdb.clone());

        let keys_to_prune = pruner
            .collect_pruning_batch::<ModuleAccessoryState>(3)
            .unwrap();
        rocksdb.write_schemas(&keys_to_prune).unwrap();

        // Assert that keys are deleted
        // Key A: has versions [6,7,8,9] -> keep [7,8,9], prune [6]
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"A".to_vec(), SlotNumber::new(6)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"A".to_vec(), SlotNumber::new(7)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"A".to_vec(), SlotNumber::new(8)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"A".to_vec(), SlotNumber::new(9)))
            .unwrap()
            .is_some());

        // Key B: has versions [7,8,9] -> keep all (only 3 versions)
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"B".to_vec(), SlotNumber::new(7)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"B".to_vec(), SlotNumber::new(8)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"B".to_vec(), SlotNumber::new(9)))
            .unwrap()
            .is_some());

        // Key C: has versions [0,1,2,3,4,5,6,7,8,9] -> keep [7,8,9], prune [0,1,2,3,4,5,6]
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(0)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(1)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(2)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(3)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(4)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(5)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(6)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(7)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(8)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"C".to_vec(), SlotNumber::new(9)))
            .unwrap()
            .is_some());

        // Key D: has versions [0,3,6,9] -> keep [3,6,9], prune [0]
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"D".to_vec(), SlotNumber::new(0)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"D".to_vec(), SlotNumber::new(3)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"D".to_vec(), SlotNumber::new(6)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"D".to_vec(), SlotNumber::new(9)))
            .unwrap()
            .is_some());

        // Key E: has versions [0,1,2] -> keep all (only 3 versions)
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"E".to_vec(), SlotNumber::new(0)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"E".to_vec(), SlotNumber::new(1)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"E".to_vec(), SlotNumber::new(2)))
            .unwrap()
            .is_some());

        // Key F: has versions [0,1,2,3] -> keep [1,2,3], prune [0]
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"F".to_vec(), SlotNumber::new(0)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"F".to_vec(), SlotNumber::new(1)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"F".to_vec(), SlotNumber::new(2)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"F".to_vec(), SlotNumber::new(3)))
            .unwrap()
            .is_some());

        // Key G: has versions [0,7,8,9] -> keep [7,8,9], prune [0]
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"G".to_vec(), SlotNumber::new(0)))
            .unwrap()
            .is_none());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"G".to_vec(), SlotNumber::new(7)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"G".to_vec(), SlotNumber::new(8)))
            .unwrap()
            .is_some());
        assert!(rocksdb
            .get::<ModuleAccessoryState>(&(b"G".to_vec(), SlotNumber::new(9)))
            .unwrap()
            .is_some());
    }
}
