use sha2::{Digest, Sha256};

pub type DefaultHash = Sha2Hash;

/// The output of a sha2-256 hash
///
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Sha2Hash(pub [u8; 32]);

impl AsRef<[u8]> for Sha2Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

pub fn sha2(item: &[u8]) -> Sha2Hash {
    let mut hasher = Sha256::new();
    hasher.update(item);

    let mut output = Sha2Hash::default();
    output.0.copy_from_slice(&hasher.finalize()[..]);
    output
}

pub fn sha2_merkle(l: &[u8], r: &[u8]) -> Sha2Hash {
    let mut hasher = Sha256::new();
    hasher.update(l);
    hasher.update(r);

    let mut output = Sha2Hash::default();
    output.0.copy_from_slice(&hasher.finalize()[..]);
    output
}

#[test]
fn test_sha2() {
    let res = sha2(b"hello world");
    assert_eq!(
        res.0,
        [
            185, 77, 39, 185, 147, 77, 62, 8, 165, 46, 82, 215, 218, 125, 171, 250, 196, 132, 239,
            227, 122, 83, 128, 238, 144, 136, 247, 172, 226, 239, 205, 233
        ]
    )
}
