use super::{errors::CodecError, ColumnFamilyName, Schema, ValueCodec};
use super::{KeyDecoder, KeyEncoder, Result};
use std::fmt::Debug;

use crate::serial::Decode;
use crate::services::da::SlotData;

pub const SLOT_CF_NAME: ColumnFamilyName = "slot";
pub type SlotNumber = u64;

#[derive(Debug)]
pub struct SlotSchema<T>(std::marker::PhantomData<T>);

impl<T, E> Schema for SlotSchema<T>
where
    CodecError: From<E>,
    T: SlotData + Send + Sync + 'static + Decode<Error = E>,
{
    type Key = SlotNumber;
    type Value = T;

    const COLUMN_FAMILY_NAME: ColumnFamilyName = SLOT_CF_NAME;
}

impl<T, E> KeyEncoder<SlotSchema<T>> for SlotNumber
where
    CodecError: From<E>,
    T: SlotData + Send + Sync + 'static + Decode<Error = E>,
{
    fn encode_key(&self) -> super::Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }
}

impl<T, E> KeyDecoder<SlotSchema<T>> for SlotNumber
where
    CodecError: From<E>,
    T: SlotData + Send + Sync + 'static + Decode<Error = E>,
{
    fn decode_key(data: &[u8]) -> super::Result<Self> {
        if data.len() != 8 {
            return Err(CodecError::InvalidKeyLength {
                expected: 8,
                got: data.len(),
            });
        }
        let bytes: [u8; 8] = data.try_into().unwrap();
        Ok(u64::from_be_bytes(bytes))
    }
}

impl<T, E> ValueCodec<SlotSchema<T>> for T
where
    CodecError: From<E>,
    T: SlotData + Decode<Error = E> + Send + Sync + 'static,
{
    fn encode_value(&self) -> Result<Vec<u8>> {
        Ok(self.encode_to_vec())
    }

    fn decode_value(mut data: &[u8]) -> Result<Self> {
        Ok(<T as Decode>::decode(&mut data)?)
    }
}
