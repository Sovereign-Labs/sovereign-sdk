use std::collections::{btree_map, BTreeMap, HashMap};
use std::iter::Rev;

use crate::metrics::SCHEMADB_BATCH_PUT_LATENCY_SECONDS;
use crate::schema::{ColumnFamilyName, KeyCodec, ValueCodec};
use crate::{Operation, Schema, SchemaKey};

// [`SchemaBatch`] holds a collection of updates that can be applied to a DB
/// ([`Schema`]) atomically. The updates will be applied in the order in which
/// they are added to the [`SchemaBatch`].
#[derive(Debug, Default)]
pub struct SchemaBatch {
    // Temporary pub(crate), before iterator is done
    pub(crate) last_writes: HashMap<ColumnFamilyName, BTreeMap<SchemaKey, Operation>>,
}

impl SchemaBatch {
    /// Creates an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an insert/update operation to the batch.
    pub fn put<S: Schema>(
        &mut self,
        key: &impl KeyCodec<S>,
        value: &impl ValueCodec<S>,
    ) -> anyhow::Result<()> {
        let _timer = SCHEMADB_BATCH_PUT_LATENCY_SECONDS
            .with_label_values(&["unknown"])
            .start_timer();
        let key = key.encode_key()?;
        let put_operation = Operation::Put {
            value: value.encode_value()?,
        };
        self.insert_operation::<S>(key, put_operation);
        Ok(())
    }

    /// Adds a delete operation to the batch.
    pub fn delete<S: Schema>(&mut self, key: &impl KeyCodec<S>) -> anyhow::Result<()> {
        let key = key.encode_key()?;
        self.insert_operation::<S>(key, Operation::Delete);

        Ok(())
    }

    fn insert_operation<S: Schema>(&mut self, key: SchemaKey, operation: Operation) {
        let column_writes = self.last_writes.entry(S::COLUMN_FAMILY_NAME).or_default();
        column_writes.insert(key, operation);
    }

    pub(crate) fn read<S: Schema>(
        &self,
        key: &impl KeyCodec<S>,
    ) -> anyhow::Result<Option<&Operation>> {
        let key = key.encode_key()?;
        if let Some(column_writes) = self.last_writes.get(&S::COLUMN_FAMILY_NAME) {
            return Ok(column_writes.get(&key));
        }
        Ok(None)
    }

    /// Iterate over all the writes in the batch for a given column family in reversed lexicographic order
    pub fn iter<S: Schema>(&self) -> SchemaBatchIterator<'_, S> {
        let rows = self.last_writes.get(&S::COLUMN_FAMILY_NAME).unwrap();
        SchemaBatchIterator {
            inner: rows.iter().rev(),
            _phantom: std::marker::PhantomData,
        }
    }
    //
    // pub fn min<S: Schema>(&self) -> Option<(&SchemaKey, &Operation)> {
    //     self.last_writes
    //         .get(&S::COLUMN_FAMILY_NAME)
    //         .unwrap()
    //         .first_key_value()
    // }
    //
    // pub fn max<S: Schema>(&self) -> Option<(&SchemaKey, &Operation)> {
    //     self.last_writes
    //         .get(&S::COLUMN_FAMILY_NAME)
    //         .unwrap()
    //         .last_key_value()
    // }
}

/// Iterator over [`SchemaBatch`] for a given column family in reversed lexicographic order
pub struct SchemaBatchIterator<'a, S: Schema> {
    inner: Rev<btree_map::Iter<'a, SchemaKey, Operation>>,
    _phantom: std::marker::PhantomData<S>,
}

impl<'a, S> Iterator for SchemaBatchIterator<'a, S>
where
    S: Schema,
{
    type Item = (&'a SchemaKey, &'a Operation);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

#[cfg(feature = "arbitrary")]
impl proptest::arbitrary::Arbitrary for SchemaBatch {
    type Parameters = &'static [ColumnFamilyName];
    fn arbitrary_with(columns: Self::Parameters) -> Self::Strategy {
        use proptest::prelude::any;
        use proptest::strategy::Strategy;

        proptest::collection::vec(any::<BTreeMap<SchemaKey, Operation>>(), columns.len())
            .prop_map::<SchemaBatch, _>(|vec_vec_write_ops| {
                let mut rows = HashMap::new();
                for (col, write_op) in columns.iter().zip(vec_vec_write_ops.into_iter()) {
                    rows.insert(*col, write_op);
                }
                SchemaBatch { last_writes: rows }
            })
            .boxed()
    }

    type Strategy = proptest::strategy::BoxedStrategy<Self>;
}
