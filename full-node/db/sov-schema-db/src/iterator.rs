use std::iter::FusedIterator;
use std::marker::PhantomData;

use anyhow::Result;

use crate::metrics::{SCHEMADB_ITER_BYTES, SCHEMADB_ITER_LATENCY_SECONDS};
use crate::schema::{KeyDecoder, Schema, ValueCodec};
use crate::{SchemaKey, SchemaValue};

/// This defines a type that can be used to seek a [`SchemaIterator`], via
/// interfaces like [`SchemaIterator::seek`]. Mind you, not all
/// [`KeyEncoder`](crate::schema::KeyEncoder)s shall be [`SeekKeyEncoder`]s, and
/// vice versa. E.g.:
///
/// - Some key types don't use an encoding that results in sensible
/// seeking behavior under lexicographic ordering (what RocksDB uses by
/// default), which means you shouldn't implement [`SeekKeyEncoder`] at all.
/// - Other key types might maintain full lexicographic order, which means the
/// original key type can also be [`SeekKeyEncoder`].
/// - Other key types may be composite, and the first field alone may be
/// a good candidate for [`SeekKeyEncoder`].
pub trait SeekKeyEncoder<S: Schema + ?Sized>: Sized {
    /// Converts `self` to bytes which is used to seek the underlying raw
    /// iterator.
    ///
    /// If `self` is also a [`KeyEncoder`](crate::schema::KeyEncoder), then
    /// [`SeekKeyEncoder::encode_seek_key`] MUST return the same bytes as
    /// [`KeyEncoder::encode_key`](crate::schema::KeyEncoder::encode_key).
    fn encode_seek_key(&self) -> crate::schema::Result<Vec<u8>>;
}

pub(crate) enum ScanDirection {
    Forward,
    Backward,
}

/// DB Iterator parameterized on [`Schema`] that seeks with [`Schema::Key`] and yields
/// [`Schema::Key`] and [`Schema::Value`] pairs.
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

    /// Reverses iterator direction.
    pub fn rev(self) -> Self {
        let new_direction = match self.direction {
            ScanDirection::Forward => ScanDirection::Backward,
            ScanDirection::Backward => ScanDirection::Forward,
        };
        SchemaIterator {
            db_iter: self.db_iter,
            direction: new_direction,
            phantom: Default::default(),
        }
    }

    fn next_impl(&mut self) -> Result<Option<IteratorOutput<S::Key, S::Value>>> {
        let _timer = SCHEMADB_ITER_LATENCY_SECONDS
            .with_label_values(&[S::COLUMN_FAMILY_NAME])
            .start_timer();

        if !self.db_iter.valid() {
            self.db_iter.status()?;
            return Ok(None);
        }

        let raw_key = self.db_iter.key().expect("db_iter.key() failed.");
        let raw_value = self.db_iter.value().expect("db_iter.value() failed.");
        let value_size_bytes = raw_value.len();
        SCHEMADB_ITER_BYTES
            .with_label_values(&[S::COLUMN_FAMILY_NAME])
            .observe((raw_key.len() + raw_value.len()) as f64);

        let key = <S::Key as KeyDecoder<S>>::decode_key(raw_key)?;
        let value = <S::Value as ValueCodec<S>>::decode_value(raw_value)?;

        match self.direction {
            ScanDirection::Forward => self.db_iter.next(),
            ScanDirection::Backward => self.db_iter.prev(),
        }

        Ok(Some(IteratorOutput {
            key,
            value,
            value_size_bytes,
        }))
    }
}

/// The output of [`SchemaIterator`]'s next_impl
pub struct IteratorOutput<K, V> {
    pub key: K,
    pub value: V,
    pub value_size_bytes: usize,
}

impl<K, V> IteratorOutput<K, V> {
    pub fn into_tuple(self) -> (K, V) {
        (self.key, self.value)
    }
}

impl<'a, S> Iterator for SchemaIterator<'a, S>
where
    S: Schema,
{
    type Item = Result<IteratorOutput<S::Key, S::Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_impl().transpose()
    }
}

impl<'a, S> FusedIterator for SchemaIterator<'a, S> where S: Schema {}

/// Iterates over given column backwards
pub struct RawDbReverseIterator<'a> {
    db_iter: rocksdb::DBRawIterator<'a>,
}

impl<'a> RawDbReverseIterator<'a> {
    pub(crate) fn new(mut db_iter: rocksdb::DBRawIterator<'a>) -> Self {
        db_iter.seek_to_last();
        RawDbReverseIterator { db_iter }
    }

    /// Navigate iterator go given key
    pub fn seek(&mut self, seek_key: SchemaKey) -> Result<()> {
        self.db_iter.seek_for_prev(&seek_key);
        Ok(())
    }
}

impl<'a> Iterator for RawDbReverseIterator<'a> {
    type Item = (SchemaKey, SchemaValue);

    fn next(&mut self) -> Option<Self::Item> {
        if !self.db_iter.valid() {
            self.db_iter.status().ok()?;
            return None;
        }

        let next_item = self.db_iter.item().expect("db_iter.key() failed.");
        // Have to allocate to fix lifetime issue
        let next_item = (next_item.0.to_vec(), next_item.1.to_vec());

        self.db_iter.prev();

        Some(next_item)
    }
}
