use crate::{Address, AddressBech32};

impl serde::Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&AddressBech32::from(self), serializer)
        } else {
            serde::Serialize::serialize(&self.addr, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let address_bech32: AddressBech32 = serde::Deserialize::deserialize(deserializer)?;
            Ok(Address::from(address_bech32.to_byte_array()))
        } else {
            let addr = <[u8; 32] as serde::Deserialize>::deserialize(deserializer)?;
            Ok(Address { addr })
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_address_serialization() {
        let address = Address::from([11; 32]);
        let data: String = serde_json::to_string(&address).unwrap();
        let deserialized_address = serde_json::from_str::<Address>(&data).unwrap();

        assert_eq!(address, deserialized_address);
        assert_eq!(
            deserialized_address.to_string(),
            "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9stup8tx"
        );
    }
}
