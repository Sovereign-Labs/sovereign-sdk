//! Defines namespaces that are used to partition the state of the rollup.

use core::fmt::Debug;

use rockbound::schema::{ColumnFamilyName, KeyDecoder, KeyEncoder, ValueCodec};
use rockbound::versioned_db::SchemaWithVersion;
use rockbound::{CodecError, Schema};

use crate::schema::types::slot_key::{SlotKey, SlotValue};

/// Nomt state values for current state.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NomtStateValues<N: Namespace>(std::marker::PhantomData<N>);
/// The generic Namespace trait used across the rollup to select a given state partition.
/// We need to define the constants by hand because currently, fully generic expression resolution
/// in constants is unstable: `<https://github.com/rust-lang/rust/issues/76560>`
pub trait Namespace: Sync + Send + Debug + Clone + Copy + 'static + Default {
    /// Mapping table from (key, version) to state value.
    /// Static name used to define the table.
    const STATE_VALUES_TABLE_NAME: ColumnFamilyName;

    /// The column family used for pruning.
    const PRUNING_COLUMN_FAMILY: ColumnFamilyName;

    /// The column family used for committed versions.
    const VERSION_METADATA_COLUMN: ColumnFamilyName;

    /// The column family used for historical data.
    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName;
}

impl<N: Namespace> Schema for NomtStateValues<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::STATE_VALUES_TABLE_NAME;

    type Key = SlotKey;
    type Value = SlotValue;
}

impl<N: Namespace> SchemaWithVersion for NomtStateValues<N> {
    const HISTORICAL_COLUMN_FAMILY_NAME: ColumnFamilyName = N::HISTORICAL_COLUMN_FAMILY;
    const PRUNING_COLUMN_FAMILY_NAME: ColumnFamilyName = N::PRUNING_COLUMN_FAMILY;
    const VERSION_METADATA_COLUMN_FAMILY_NAME: ColumnFamilyName = N::VERSION_METADATA_COLUMN;
}

impl<N: Namespace> KeyEncoder<NomtStateValues<N>> for SlotKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        // SchemaKey is already a borsh-encoded value, so we just copy the bytes
        Ok(self.as_ref().to_vec())
    }
}

impl<N: Namespace> KeyDecoder<NomtStateValues<N>> for SlotKey {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        if data.len() < 2 {
            return Err(CodecError::InvalidKeyLength {
                expected: 2,
                got: data.len(),
            });
        }
        Ok(SlotKey::from_slice_including_prefix(data))
    }
}

impl<N: Namespace> ValueCodec<NomtStateValues<N>> for SlotValue {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ref().to_vec())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(SlotValue::from(data.to_vec()))
    }
}
