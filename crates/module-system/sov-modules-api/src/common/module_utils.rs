use std::borrow::Cow;

use schemars::r#gen::SchemaGenerator;
use schemars::schema::{InstanceType, Schema, SchemaObject};
use schemars::JsonSchema;

/// We may use [`NotInstantiable`] type for modules that do not support calls.
///
/// ## Details
/// This is a simple struct that implements all the necessary call message traits
/// and that can be used as a placeholder for modules that do not support calls.
#[derive(
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    crate::macros::UniversalWallet,
)]
#[universal_wallet(sov_modules_api_path = crate)]
pub enum NotInstantiable {}

impl borsh::BorshDeserialize for NotInstantiable {
    // It is impossible to deserialize to NotInstantiable.
    fn deserialize_reader<R: std::io::prelude::Read>(
        _reader: &mut R,
    ) -> Result<Self, std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "NotInstantiable is not instantiable",
        ))
    }
}

impl borsh::BorshSerialize for NotInstantiable {
    // Since it impossible to have a value of NotInstantiable this code is unreachable.
    fn serialize<W: std::io::Write>(&self, _writer: &mut W) -> Result<(), std::io::Error> {
        unreachable!()
    }
}

// Override the jsonschema to be null rather than an enum with no variants. This allows quicktype to handle this type.
impl JsonSchema for NotInstantiable {
    fn schema_name() -> String {
        "NotInstantiable".to_owned()
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("NotInstantiable")
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        SchemaObject {
            instance_type: Some(InstanceType::Null.into()),
            format: None,
            ..Default::default()
        }
        .into()
    }
}
