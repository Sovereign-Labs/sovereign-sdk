use super::{errors::CodecError, ColumnFamilyName, Schema, ValueCodec};
use super::{KeyDecoder, KeyEncoder, Result};
use std::fmt::Debug;

use crate::services::da::SlotData;

pub const SLOT_CF_NAME: ColumnFamilyName = "slot";
pub type SlotNumber = u64;

#[derive(Debug)]
pub struct SlotSchema<T>(std::marker::PhantomData<T>);

impl<T> Schema for SlotSchema<T>
where
    T: SlotData + Send + Sync + 'static,
{
    const COLUMN_FAMILY_NAME: ColumnFamilyName = SLOT_CF_NAME;
    type Key = SlotNumber;

    type Value = T;
}

impl<T> KeyEncoder<SlotSchema<T>> for SlotNumber
where
    T: SlotData + Send + Sync + 'static,
{
    fn encode_key(&self) -> Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }
}

impl<T> KeyDecoder<SlotSchema<T>> for SlotNumber
where
    T: SlotData + Send + Sync + 'static,
{
    fn decode_key(data: &[u8]) -> Result<Self> {
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

impl<T> ValueCodec<SlotSchema<T>> for T
where
    T: SlotData + Send + Sync + 'static,
{
    fn encode_value(&self) -> Result<Vec<u8>> {
        self.try_to_vec().map_err(|e| e.into())
    }

    fn decode_value(mut data: &[u8]) -> Result<Self> {
        T::deserialize(&mut data).map_err(|e| e.into())
    }
}
