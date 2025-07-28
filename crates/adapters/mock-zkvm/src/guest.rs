use serde::Serialize;

use crate::MockZkVerifier;
/// A mock implementing the Guest.
#[derive(Default)]
pub struct MockZkGuest {}

impl sov_rollup_interface::zk::ZkvmGuest for MockZkGuest {
    type Verifier = MockZkVerifier;
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        unimplemented!()
    }

    fn commit<T: Serialize>(&self, _item: &T) {
        unimplemented!()
    }
}
