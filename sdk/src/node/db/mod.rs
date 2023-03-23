// Adapted from Aptos::storage::schemadb;
// While most of the Sovereign SDK will be available under both
// MIT and APACHE 2.0 licenses, this file is
// licensed under APACHE 2.0 only.
use std::fmt::Debug;

use crate::services::da::SlotData;

use self::errors::CodecError;
pub mod errors;
mod slot;
pub use slot::*;
mod slot_by_hash;
pub use slot_by_hash::*;

pub trait SlotStore {
    type Slot: SlotData;
    fn get(&self, hash: &[u8; 32]) -> Option<Self::Slot>;
    fn insert(&self, hash: [u8; 32], slot_data: Self::Slot);
}
pub type ColumnFamilyName = &'static str;

/// This trait defines a schema: an association of a column family name, the key type and the value
/// type.
pub trait Schema: Debug + Send + Sync + 'static + Sized {
    /// The column family name associated with this struct.
    /// Note: all schemas within the same SchemaDB must have distinct column family names.
    const COLUMN_FAMILY_NAME: ColumnFamilyName;

    /// Type of the key.
    type Key: KeyCodec<Self>;

    /// Type of the value.
    type Value: ValueCodec<Self>;
}
pub type Result<T, E = CodecError> = core::result::Result<T, E>;

/// This trait defines a type that can serve as a [`Schema::Key`].
pub trait KeyCodec<S: Schema + ?Sized>: KeyEncoder<S> + KeyDecoder<S> {}

impl<T, S: Schema + ?Sized> KeyCodec<S> for T where T: KeyEncoder<S> + KeyDecoder<S> {}

pub trait KeyEncoder<S: Schema + ?Sized>: Sized + PartialEq + Debug {
    /// Converts `self` to bytes to be stored in DB.
    fn encode_key(&self) -> Result<Vec<u8>>;
}

pub trait KeyDecoder<S: Schema + ?Sized>: Sized + PartialEq + Debug {
    /// Converts bytes fetched from DB to `Self`.
    fn decode_key(data: &[u8]) -> Result<Self>;
}

/// This trait defines a type that can serve as a [`Schema::Value`].
pub trait ValueCodec<S: Schema + ?Sized>: Sized + PartialEq + Debug {
    /// Converts `self` to bytes to be stored in DB.
    fn encode_value(&self) -> Result<Vec<u8>>;
    /// Converts bytes fetched from DB to `Self`.
    fn decode_value(data: &[u8]) -> Result<Self>;
}

/// This defines a type that can be used to seek a [`SchemaIterator`](crate::SchemaIterator), via
/// interfaces like [`seek`](crate::SchemaIterator::seek).
pub trait SeekKeyEncoder<S: Schema + ?Sized>: Sized {
    /// Converts `self` to bytes which is used to seek the underlying raw iterator.
    fn encode_seek_key(&self) -> Result<Vec<u8>>;
}

/// All keys can automatically be used as seek keys.
impl<S, K> SeekKeyEncoder<S> for K
where
    S: Schema,
    K: KeyEncoder<S>,
{
    /// Delegates to [`KeyCodec::encode_key`].
    fn encode_seek_key(&self) -> Result<Vec<u8>> {
        <K as KeyEncoder<S>>::encode_key(self)
    }
}

#[macro_export]
macro_rules! define_schema {
    ($schema_type:ident, $key_type:ty, $value_type:ty, $cf_name:expr) => {
        #[derive(Debug)]
        pub(crate) struct $schema_type;

        impl $crate::db::Schema for $schema_type {
            type Key = $key_type;
            type Value = $value_type;

            const COLUMN_FAMILY_NAME: $crate::db::ColumnFamilyName = $cf_name;
        }
    };
}

#[cfg(feature = "fuzzing")]
pub mod fuzzing {
    use super::{KeyDecoder, KeyEncoder, Schema, ValueCodec};
    use proptest::{collection::vec, prelude::*};

    /// Helper used in tests to assert a (key, value) pair for a certain [`Schema`] is able to convert
    /// to bytes and convert back.
    pub fn assert_encode_decode<S: Schema>(key: &S::Key, value: &S::Value) {
        {
            let encoded = key.encode_key().expect("Encoding key should work.");
            let decoded = S::Key::decode_key(&encoded).expect("Decoding key should work.");
            assert_eq!(*key, decoded);
        }
        {
            let encoded = value.encode_value().expect("Encoding value should work.");
            let decoded = S::Value::decode_value(&encoded).expect("Decoding value should work.");
            assert_eq!(*value, decoded);
        }
    }

    /// Helper used in tests and fuzzers to make sure a schema never panics when decoding random bytes.
    #[allow(unused_must_use)]
    pub fn assert_no_panic_decoding<S: Schema>(bytes: &[u8]) {
        S::Key::decode_key(bytes);
        S::Value::decode_value(bytes);
    }

    pub fn arb_small_vec_u8() -> impl Strategy<Value = Vec<u8>> {
        vec(any::<u8>(), 0..2048)
    }

    #[macro_export]
    macro_rules! test_no_panic_decoding {
        ($schema_type:ty) => {
            use proptest::prelude::*;
            use schemadb::schema::fuzzing::{arb_small_vec_u8, assert_no_panic_decoding};

            proptest! {
                #[test]
                fn test_no_panic_decoding(bytes in arb_small_vec_u8()) {
                    assert_no_panic_decoding::<$schema_type>(&bytes)
                }
            }
        };
    }
}
