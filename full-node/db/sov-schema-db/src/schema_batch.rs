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
    /// Returns None column family name does not have any writes
    pub fn iter<S: Schema>(
        &self,
    ) -> SchemaBatchIterator<'_, S, Rev<btree_map::Iter<SchemaKey, Operation>>> {
        let some_rows = self.last_writes.get(&S::COLUMN_FAMILY_NAME);
        SchemaBatchIterator {
            inner: some_rows.map(|rows| rows.iter().rev()),
            _phantom_schema: std::marker::PhantomData,
        }
    }

    /// Return iterator that iterates from operations with largest_key == upper_bound backwards
    pub fn iter_range<S: Schema>(
        &self,
        upper_bound: SchemaKey,
    ) -> SchemaBatchIterator<'_, S, Rev<btree_map::Range<SchemaKey, Operation>>> {
        let some_rows = self.last_writes.get(&S::COLUMN_FAMILY_NAME);
        SchemaBatchIterator {
            inner: some_rows.map(|rows| rows.range(..=upper_bound).rev()),
            _phantom_schema: std::marker::PhantomData,
        }
    }

    pub(crate) fn merge(&mut self, other: SchemaBatch) {
        for (cf_name, other_cf_map) in other.last_writes {
            let self_cf_map = self.last_writes.entry(cf_name).or_default();

            for (key, operation) in other_cf_map {
                self_cf_map.insert(key, operation);
            }
        }
    }
}

/// Iterator over [`SchemaBatch`] for a given column family in reversed lexicographic order
pub struct SchemaBatchIterator<'a, S, I>
where
    S: Schema,
    I: Iterator<Item = (&'a SchemaKey, &'a Operation)>,
{
    inner: Option<I>,
    _phantom_schema: std::marker::PhantomData<S>,
}

impl<'a, S, I> Iterator for SchemaBatchIterator<'a, S, I>
where
    S: Schema,
    I: Iterator<Item = (&'a SchemaKey, &'a Operation)>,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.as_mut().and_then(|inner| inner.next())
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
