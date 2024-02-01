//! Helpers structures for testing, such as fields

use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use crate::schema::{KeyDecoder, KeyEncoder, ValueCodec};
use crate::{CodecError, Schema, SeekKeyEncoder};

#[derive(Debug, Eq, PartialEq, Clone)]
/// Key that composed out of tuple of r u32
pub struct TestCompositeField(pub u32, pub u32, pub u32);

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
/// Simple value around u32
pub struct TestField(pub u32);

impl<S: Schema> KeyEncoder<S> for TestCompositeField {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut bytes = vec![];
        bytes
            .write_u32::<BigEndian>(self.0)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.1)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.2)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        Ok(bytes)
    }
}

impl<S: Schema> KeyDecoder<S> for TestCompositeField {
    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let mut reader = std::io::Cursor::new(data);
        Ok(TestCompositeField(
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

impl<S: Schema> SeekKeyEncoder<S> for TestCompositeField {
    fn encode_seek_key(&self) -> crate::schema::Result<Vec<u8>> {
        <TestCompositeField as KeyEncoder<S>>::encode_key(self)
    }
}

impl TestField {
    fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().to_vec()
    }

    fn from_bytes(data: &[u8]) -> std::result::Result<Self, CodecError> {
        let mut reader = std::io::Cursor::new(data);
        Ok(TestField(
            reader
                .read_u32::<BigEndian>()
                .map_err(|e| CodecError::Wrapped(e.into()))?,
        ))
    }
}

impl<S: Schema> ValueCodec<S> for TestField {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_bytes(data)
    }
}

impl<S: Schema> KeyDecoder<S> for TestField {
    fn decode_key(data: &[u8]) -> std::result::Result<Self, CodecError> {
        Self::from_bytes(data)
    }
}

impl<S: Schema> KeyEncoder<S> for TestField {
    fn encode_key(&self) -> std::result::Result<Vec<u8>, CodecError> {
        Ok(self.as_bytes())
    }
}

/// KeyPrefix over single u32
pub struct KeyPrefix1(pub u32);

impl<S: Schema> SeekKeyEncoder<S> for KeyPrefix1 {
    fn encode_seek_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0.to_be_bytes().to_vec())
    }
}

/// KeyPrefix over pair of u32
pub struct KeyPrefix2(pub u32, pub u32);

impl<S: Schema> SeekKeyEncoder<S> for KeyPrefix2 {
    fn encode_seek_key(&self) -> Result<Vec<u8>, CodecError> {
        let mut bytes = vec![];
        bytes
            .write_u32::<BigEndian>(self.0)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        bytes
            .write_u32::<BigEndian>(self.1)
            .map_err(|e| CodecError::Wrapped(e.into()))?;
        Ok(bytes)
    }
}
