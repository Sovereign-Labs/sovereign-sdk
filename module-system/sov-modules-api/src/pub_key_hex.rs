use derive_more::Display;

use crate::default_signature::DefaultPublicKey;
#[derive(
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Clone,
    Eq,
    Display,
)]
#[serde(try_from = "String", into = "String")]
#[display(fmt = "{}", "hex")]
pub struct PublicKeyHex {
    hex: String,
}

impl PublicKeyHex {
    pub fn new(hex: String) -> Self {
        todo!();
    }
}

impl TryFrom<String> for PublicKeyHex {
    type Error = String;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl From<PublicKeyHex> for String {
    fn from(value: PublicKeyHex) -> Self {
        todo!()
    }
}

impl TryFrom<DefaultPublicKey> for PublicKeyHex {
    type Error = String;

    fn try_from(value: DefaultPublicKey) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl TryFrom<PublicKeyHex> for DefaultPublicKey {
    type Error = String;

    fn try_from(value: PublicKeyHex) -> Result<Self, Self::Error> {
        todo!()
    }
}
