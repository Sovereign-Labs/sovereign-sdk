use anyhow::Result;
use std::marker::PhantomData;

use sovereign_sdk::db::{KeyDecoder, Schema, SeekKeyEncoder, ValueCodec};

use crate::metrics::{SCHEMADB_ITER_BYTES, SCHEMADB_ITER_LATENCY_SECONDS};

pub enum ScanDirection {
    Forward,
    Backward,
}

/// DB Iterator parameterized on [`Schema`] that seeks with [`Schema::Key`] and yields
/// [`Schema::Key`] and [`Schema::Value`]
pub struct SchemaIterator<'a, S> {
    db_iter: rocksdb::DBRawIterator<'a>,
    direction: ScanDirection,
    phantom: PhantomData<S>,
}

impl<'a, S> SchemaIterator<'a, S>
where
    S: Schema,
{
    pub(crate) fn new(db_iter: rocksdb::DBRawIterator<'a>, direction: ScanDirection) -> Self {
        SchemaIterator {
            db_iter,
            direction,
            phantom: PhantomData,
        }
    }

    /// Seeks to the first key.
    pub fn seek_to_first(&mut self) {
        self.db_iter.seek_to_first();
    }

    /// Seeks to the last key.
    pub fn seek_to_last(&mut self) {
        self.db_iter.seek_to_last();
    }

    /// Seeks to the first key whose binary representation is equal to or greater than that of the
    /// `seek_key`.
    pub fn seek(&mut self, seek_key: &impl SeekKeyEncoder<S>) -> Result<()> {
        let key = seek_key.encode_seek_key()?;
        self.db_iter.seek(&key);
        Ok(())
    }

    /// Seeks to the last key whose binary representation is less than or equal to that of the
    /// `seek_key`.
    ///
    /// See example in [`RocksDB doc`](https://github.com/facebook/rocksdb/wiki/SeekForPrev).
    pub fn seek_for_prev(&mut self, seek_key: &impl SeekKeyEncoder<S>) -> Result<()> {
        let key = seek_key.encode_seek_key()?;
        self.db_iter.seek_for_prev(&key);
        Ok(())
    }

    fn next_impl(&mut self) -> Result<Option<(S::Key, S::Value)>> {
        let _timer = SCHEMADB_ITER_LATENCY_SECONDS
            .with_label_values(&[S::COLUMN_FAMILY_NAME])
            .start_timer();

        if !self.db_iter.valid() {
            self.db_iter.status()?;
            return Ok(None);
        }

        let raw_key = self.db_iter.key().expect("db_iter.key() failed.");
        let raw_value = self.db_iter.value().expect("db_iter.value() failed.");
        SCHEMADB_ITER_BYTES
            .with_label_values(&[S::COLUMN_FAMILY_NAME])
            .observe((raw_key.len() + raw_value.len()) as f64);

        let key = <S::Key as KeyDecoder<S>>::decode_key(raw_key)?;
        let value = <S::Value as ValueCodec<S>>::decode_value(raw_value)?;

        match self.direction {
            ScanDirection::Forward => self.db_iter.next(),
            ScanDirection::Backward => self.db_iter.prev(),
        }

        Ok(Some((key, value)))
    }
}

impl<'a, S> Iterator for SchemaIterator<'a, S>
where
    S: Schema,
{
    type Item = Result<(S::Key, S::Value)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_impl().transpose()
    }
}
