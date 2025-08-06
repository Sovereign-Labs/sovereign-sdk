//! Defines namespaces that are used to partition the state of the rollup.

use core::fmt::Debug;
use std::io::Cursor;
use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use jmt::storage::{NibblePath, Node, NodeKey};
use rockbound::schema::{ColumnFamilyName, KeyDecoder, KeyEncoder, ValueCodec};
use rockbound::versioned_db::{
    PrunableKey, SchemaWithVersion, VersionedKey, VersionedTableMetadataKey,
};
use rockbound::{CodecError, Schema, SchemaKey, SchemaValue, SeekKeyEncoder};
use sov_rollup_interface::common::SlotNumber;

/// Mapping table from key Hash to jmt key
#[derive(Debug, Default)]
pub(crate) struct KeyHashToKey<N: Namespace>(std::marker::PhantomData<N>);
/// In the case of jmt, it maps key hash to node value
/// In other cases, such as nomt, it maps key to value and used for historical data.
#[derive(Debug, Default)]
pub(crate) struct StateValues<N: Namespace>(std::marker::PhantomData<N>);
/// Mapping table from (key, version) to jmt value
#[derive(Debug, Default)]
pub(crate) struct JmtNodes<N: Namespace>(std::marker::PhantomData<N>);
/// Nomt state values for current state.
#[derive(Clone, Debug, Default)]
pub struct NomtStateValues<N: Namespace>(std::marker::PhantomData<N>);
/// Nomt state values for historical data.
#[derive(Clone, Debug, Default)]
pub struct NomtHistoricalState<N: Namespace>(std::marker::PhantomData<N>);
/// Nomt state values for pruning.
#[derive(Clone, Debug, Default)]
pub struct NomtPruningState<N: Namespace>(std::marker::PhantomData<N>);
/// Nomt singleton for committed version.
#[derive(Clone, Debug, Default)]
pub struct NomtCommittedVersion<N: Namespace>(std::marker::PhantomData<N>);

/// The generic Namespace trait used across the rollup to select a given state partition.
/// We need to define the constants by hand because currently, fully generic expression resolution
/// in constants is unstable: `<https://github.com/rust-lang/rust/issues/76560>`
pub trait Namespace: Sync + Send + Debug + Clone + Copy + 'static + Default {
    /// Mapping table from node hash to jmt node. Static name used to define the table.
    const KEY_HASH_TO_KEY_TABLE_NAME: ColumnFamilyName;

    /// Mapping table from node hash to jmt node. Static name used to define the table
    const JMT_NODES_TABLE_NAME: ColumnFamilyName;

    /// Mapping table from (key, version) to state value.
    /// Static name used to define the table.
    /// In the case of jmt key is actually a hash of the actual key.
    const STATE_VALUES_TABLE_NAME: ColumnFamilyName;

    /// The column family used for pruning.
    const PRUNING_COLUMN_FAMILY: ColumnFamilyName;

    /// The column family used for committed versions.
    const VERSION_METADATA_COLUMN: ColumnFamilyName;

    /// The column family used for historical data.
    const HISTORICAL_COLUMN_FAMILY: ColumnFamilyName;

    /// Returns the table names for this namespace.
    // Note: This intentionally does not include the column families for the versioned DB. Those are handled by the `SchemaWithVersion` trait.
    fn get_jmt_table_names() -> [ColumnFamilyName; 3] {
        [
            Self::KEY_HASH_TO_KEY_TABLE_NAME,
            Self::JMT_NODES_TABLE_NAME,
            Self::STATE_VALUES_TABLE_NAME,
        ]
    }
}

impl<N: Namespace> Schema for NomtStateValues<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::STATE_VALUES_TABLE_NAME;
    const SHOULD_CACHE: bool = true;

    type Key = Arc<SchemaKey>;
    type Value = Option<SchemaValue>;
}

impl<N: Namespace> SchemaWithVersion for NomtStateValues<N> {
    type HistoricalColumnFamily = NomtHistoricalState<N>;
    type PruningColumnFamily = NomtPruningState<N>;
    type VersionMetadatacolumn = NomtCommittedVersion<N>;
}

impl<N: Namespace> Schema for NomtHistoricalState<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::HISTORICAL_COLUMN_FAMILY;
    const SHOULD_CACHE: bool = false;

    type Key = VersionedKey<NomtStateValues<N>, Arc<SchemaKey>>;
    type Value = Option<SchemaValue>;
}

impl<N: Namespace> ValueCodec<NomtHistoricalState<N>> for Option<SchemaValue> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(CodecError::from)
    }
    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(Self::deserialize_reader(&mut &data[..])?)
    }
}

impl<N: Namespace> Schema for NomtPruningState<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::PRUNING_COLUMN_FAMILY;
    const SHOULD_CACHE: bool = false;

    type Key = PrunableKey<NomtStateValues<N>, Arc<SchemaKey>>;
    type Value = ();
}

impl<N: Namespace> ValueCodec<NomtPruningState<N>> for () {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(Vec::new())
    }
    fn decode_value(_data: &[u8]) -> Result<Self, CodecError> {
        Ok(())
    }
}

impl<N: Namespace> Schema for NomtCommittedVersion<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::VERSION_METADATA_COLUMN;
    const SHOULD_CACHE: bool = true;

    type Key = VersionedTableMetadataKey;
    type Value = u64;
}

impl<N: Namespace> KeyEncoder<NomtCommittedVersion<N>> for VersionedTableMetadataKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        self.encode()
    }
}

impl<N: Namespace> KeyDecoder<NomtCommittedVersion<N>> for VersionedTableMetadataKey {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        Self::decode(data)
    }
}

impl<N: Namespace> ValueCodec<NomtCommittedVersion<N>> for u64 {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(CodecError::from)
    }
    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(Self::deserialize_reader(&mut &data[..])?)
    }
}

impl<N: Namespace> KeyEncoder<NomtStateValues<N>> for SchemaKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::with_capacity(self.len() + 4);
        BorshSerialize::serialize(self, &mut out).map_err(CodecError::from)?;
        Ok(out)
    }
}

impl<N: Namespace> KeyDecoder<NomtStateValues<N>> for Arc<SchemaKey> {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let mut cursor = Cursor::new(data);
        let key = Vec::<u8>::deserialize_reader(&mut cursor)?;
        Ok(Arc::new(key))
    }
}

impl<N: Namespace> ValueCodec<NomtStateValues<N>> for Option<SchemaValue> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(CodecError::from)
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(Self::deserialize_reader(&mut &data[..])?)
    }
}

/* Generic implementations of the state table schemas for all the namespaces */

impl<N: Namespace> Schema for KeyHashToKey<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::KEY_HASH_TO_KEY_TABLE_NAME;
    const SHOULD_CACHE: bool = false;

    type Key = [u8; 32];
    type Value = SchemaKey;
}

impl<N: Namespace> Schema for StateValues<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::STATE_VALUES_TABLE_NAME;
    const SHOULD_CACHE: bool = false;

    type Key = (SchemaKey, SlotNumber);
    type Value = Option<SchemaValue>;
}

impl<N: Namespace> Schema for JmtNodes<N> {
    const COLUMN_FAMILY_NAME: ColumnFamilyName = N::JMT_NODES_TABLE_NAME;
    const SHOULD_CACHE: bool = false;

    type Key = NodeKey;
    type Value = Node;
}

impl<N: Namespace> KeyEncoder<KeyHashToKey<N>> for [u8; 32] {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(Into::into)
    }
}

impl<N: Namespace> KeyDecoder<KeyHashToKey<N>> for [u8; 32] {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(Into::into)
    }
}

impl<N: Namespace> ValueCodec<KeyHashToKey<N>> for SchemaKey {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(Into::into)
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(Into::into)
    }
}

impl<N: Namespace> KeyEncoder<JmtNodes<N>> for NodeKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        // 8 bytes for version, 4 each for the num_nibbles and bytes.len() fields, plus 1 byte per byte of nibllepath
        let mut output =
            Vec::with_capacity(8 + 4 + 4 + self.nibble_path().num_nibbles().div_ceil(2));
        let version = self.version().to_be_bytes();
        output.extend_from_slice(&version);
        BorshSerialize::serialize(&self.nibble_path(), &mut output)?;
        Ok(output)
    }
}
impl<N: Namespace> KeyDecoder<JmtNodes<N>> for NodeKey {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        if data.len() < 8 {
            return Err(CodecError::InvalidKeyLength {
                expected: 9,
                got: data.len(),
            });
        }
        let mut version = [0u8; 8];
        version.copy_from_slice(&data[..8]);
        let version = u64::from_be_bytes(version);
        let nibble_path = NibblePath::deserialize_reader(&mut &data[8..])?;
        Ok(Self::new(version, nibble_path))
    }
}

impl<N: Namespace> ValueCodec<JmtNodes<N>> for Node {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(CodecError::from)
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(Self::deserialize_reader(&mut &data[..])?)
    }
}

impl<T: Debug + PartialEq + AsRef<[u8]>, N: Namespace> KeyEncoder<StateValues<N>>
    for (T, SlotNumber)
{
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut out =
            Vec::with_capacity(self.0.as_ref().len() + std::mem::size_of::<SlotNumber>() + 8);
        BorshSerialize::serialize(self.0.as_ref(), &mut out).map_err(CodecError::from)?;
        // Write the version in big-endian order so that sorting order is based on the most-significant bytes of the key
        out.write_u64::<BigEndian>(self.1.get())
            .expect("serialization to vec is infallible");
        Ok(out)
    }
}

impl<T: AsRef<[u8]> + PartialEq + Debug, N: Namespace> SeekKeyEncoder<StateValues<N>>
    for (T, SlotNumber)
{
    fn encode_seek_key(&self) -> Result<Vec<u8>, CodecError> {
        <(T, SlotNumber) as KeyEncoder<StateValues<N>>>::encode_key(self)
    }
}

impl<N: Namespace> KeyDecoder<StateValues<N>> for (SchemaKey, SlotNumber) {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let mut cursor = Cursor::new(data);
        let key = Vec::<u8>::deserialize_reader(&mut cursor)?;
        let version = cursor.read_u64::<BigEndian>()?;
        Ok((key, SlotNumber::new_dangerous(version)))
    }
}

impl<N: Namespace> ValueCodec<StateValues<N>> for Option<SchemaValue> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        borsh::to_vec(self).map_err(CodecError::from)
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(Self::deserialize_reader(&mut &data[..])?)
    }
}
