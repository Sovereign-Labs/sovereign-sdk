use std::marker::PhantomData;

use byteorder::{BigEndian, ReadBytesExt};
use sov_schema_db::schema::{KeyDecoder, KeyEncoder, Result as CodecResult, ValueCodec};
use sov_schema_db::snapshot::{DbSnapshot, QueryManager};
use sov_schema_db::{define_schema, CodecError};
use sov_state::MerkleProofSpec;

/// Oversimplified representation of [`sov_state::ProverStorage`]
pub struct NewProverStorage<Mps: MerkleProofSpec, Q> {
    state_db: DbSnapshot<Q>,
    native_db: DbSnapshot<Q>,
    p: PhantomData<Mps>,
}

impl<Mps: MerkleProofSpec, Q: QueryManager> NewProverStorage<Mps, Q> {
    pub(crate) fn with_db_handlers(
        state_db_snapshot: DbSnapshot<Q>,
        native_db_snapshot: DbSnapshot<Q>,
    ) -> Self {
        NewProverStorage {
            state_db: state_db_snapshot,
            native_db: native_db_snapshot,
            p: Default::default(),
        }
    }

    pub(crate) fn freeze(self) -> (DbSnapshot<Q>, DbSnapshot<Q>) {
        let NewProverStorage {
            state_db,
            native_db,
            ..
        } = self;
        (state_db, native_db)
    }

    #[allow(dead_code)]
    pub(crate) fn read_state(&self, key: u64) -> anyhow::Result<Option<u64>> {
        let key = DummyField(key);
        Ok(self
            .state_db
            .read::<DummyStateSchema>(&key)?
            .map(Into::into))
    }

    #[allow(dead_code)]
    pub(crate) fn write_state(&self, key: u64, value: u64) -> anyhow::Result<()> {
        let key = DummyField(key);
        let value = DummyField(value);
        self.state_db.put::<DummyStateSchema>(&key, &value)
    }

    #[allow(dead_code)]
    pub(crate) fn read_native(&self, key: u64) -> anyhow::Result<Option<u64>> {
        let key = DummyField(key);
        Ok(self
            .native_db
            .read::<DummyNativeSchema>(&key)?
            .map(Into::into))
    }

    #[allow(dead_code)]
    pub(crate) fn write_native(&self, key: u64, value: u64) -> anyhow::Result<()> {
        let key = DummyField(key);
        let value = DummyField(value);
        self.native_db.put::<DummyNativeSchema>(&key, &value)
    }
}

// --------------
// The code below used to emulate native and state db, but on oversimplified level

pub(crate) const DUMMY_STATE_CF: &str = "DummyStateCF";
pub(crate) const DUMMY_NATIVE_CF: &str = "DummyNativeCF";

define_schema!(DummyStateSchema, DummyField, DummyField, DUMMY_STATE_CF);
define_schema!(DummyNativeSchema, DummyField, DummyField, DUMMY_NATIVE_CF);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DummyField(pub(crate) u64);

impl From<DummyField> for u64 {
    fn from(value: DummyField) -> Self {
        value.0
    }
}

impl DummyField {
    fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().to_vec()
    }

    fn from_bytes(data: &[u8]) -> CodecResult<Self> {
        let mut reader = std::io::Cursor::new(data);
        Ok(Self(
            reader
                .read_u64::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

impl KeyEncoder<DummyStateSchema> for DummyField {
    fn encode_key(&self) -> CodecResult<Vec<u8>> {
        Ok(self.as_bytes())
    }
}

impl KeyDecoder<DummyStateSchema> for DummyField {
    fn decode_key(data: &[u8]) -> CodecResult<Self> {
        Self::from_bytes(data)
    }
}

impl ValueCodec<DummyStateSchema> for DummyField {
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        Ok(self.as_bytes())
    }

    fn decode_value(data: &[u8]) -> CodecResult<Self> {
        Self::from_bytes(data)
    }
}

impl KeyEncoder<DummyNativeSchema> for DummyField {
    fn encode_key(&self) -> CodecResult<Vec<u8>> {
        Ok(self.as_bytes())
    }
}

impl KeyDecoder<DummyNativeSchema> for DummyField {
    fn decode_key(data: &[u8]) -> CodecResult<Self> {
        Self::from_bytes(data)
    }
}

impl ValueCodec<DummyNativeSchema> for DummyField {
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        Ok(self.as_bytes())
    }

    fn decode_value(data: &[u8]) -> CodecResult<Self> {
        Self::from_bytes(data)
    }
}
