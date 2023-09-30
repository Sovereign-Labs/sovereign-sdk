use ed25519_dalek::VerifyingKey as DalekPublicKey;

use crate::default_signature::DefaultPublicKey;
use crate::PublicKeyHex;

impl serde::Serialize for DefaultPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            // TODO remove clone
            serde::Serialize::serialize(&PublicKeyHex::from(self.clone()), serializer)
        } else {
            serde::Serialize::serialize(&self.pub_key, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for DefaultPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let pub_key_hex: PublicKeyHex = serde::Deserialize::deserialize(deserializer)?;
            Ok(DefaultPublicKey::try_from(&pub_key_hex).map_err(serde::de::Error::custom)?)
        } else {
            let pub_key: DalekPublicKey = serde::Deserialize::deserialize(deserializer)?;
            Ok(DefaultPublicKey { pub_key })
        }
    }
}

#[cfg(test)]
mod test {
    

    #[test]
    fn test_pub_key() {}
}
