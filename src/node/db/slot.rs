use super::Result;
use super::{errors::CodecError, ColumnFamilyName, KeyCodec, Schema, ValueCodec};
use std::fmt::Debug;

use crate::{serial::Decode, services::da::SlotData};

pub const SLOT_CF_NAME: ColumnFamilyName = "slot";
pub type SlotNumber = u64;

#[derive(Debug)]
pub struct SlotSchema<T>(std::marker::PhantomData<T>);

impl<T: Debug + Send + Sync + 'static + SlotData> Schema for SlotSchema<T> {
    type Key = SlotNumber;
    type Value = T;

    const COLUMN_FAMILY_NAME: ColumnFamilyName = SLOT_CF_NAME;
}

impl<T: Debug + Send + Sync + 'static> KeyCodec<SlotSchema<T>> for SlotNumber
where
    T: SlotData,
{
    fn encode_key(&self) -> super::Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }

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

impl<T: SlotData + Debug + Send + Sync + PartialEq + 'static> ValueCodec<SlotSchema<T>> for T
where
    T: SlotData,
{
    fn encode_value(&self) -> Result<Vec<u8>> {
        Ok(self.encode_to_vec())
    }

    fn decode_value(mut data: &[u8]) -> Result<Self> {
        Ok(<T as Decode>::decode(&mut data)?)
    }
}
