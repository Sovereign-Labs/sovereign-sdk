use crate::{SigVerificationError, Signature};
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Debug)]
pub struct DefaultPublicKey {
    pub(crate) pub_key: Vec<u8>,
}

impl DefaultPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }

    pub fn sign(&self, _msg: [u8; 32]) -> DefaultSignature {
        DefaultSignature {
            msg_sig: vec![],
            should_fail: false,
        }
    }
}

impl<T: AsRef<str>> From<T> for DefaultPublicKey {
    fn from(key: T) -> Self {
        let key = key.as_ref().as_bytes().to_vec();
        Self { pub_key: key }
    }
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq, Debug, Clone, Default)]
pub struct DefaultSignature {
    pub msg_sig: Vec<u8>,
    pub should_fail: bool,
}

impl Signature for DefaultSignature {
    type PublicKey = DefaultPublicKey;

    fn verify(
        &self,
        _pub_key: &Self::PublicKey,
        _msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError> {
        if self.should_fail {
            Err(SigVerificationError::BadSignature)
        } else {
            Ok(())
        }
    }
}
