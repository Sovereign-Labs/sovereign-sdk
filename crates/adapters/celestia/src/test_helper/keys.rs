use crate::test_helper::ADDR_1;
use crate::verifier::address::CelestiaAddress;
use std::str::FromStr;

pub struct TestCredentials {
    pub private_key_hex: String,
    pub address: CelestiaAddress,
}

pub fn credentials_1() -> TestCredentials {
    let content = include_str!("../../../../../docker/credentials/bridge-0.key");
    let private_key_hex =
        crate::da_service::keys::read_tendermint_key_file(content, "password").unwrap();
    TestCredentials {
        private_key_hex,
        address: CelestiaAddress::from_str(ADDR_1).unwrap(),
    }
}
