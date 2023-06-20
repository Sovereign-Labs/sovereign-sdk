pub const DEFAULT_LEAF_DOMAIN_SEPARATOR: [u8; 1] = [0];
pub const DEFAULT_INTERNAL_DOMAIN_SEPARATOR: [u8; 1] = [1];

pub trait MerkleHasher {
    /// The root hash for an empty tree
    fn empty_root(&mut self) -> [u8; 32];

    /// Hash a leaf node to create an inner node. This function *should* prepend a domain separator to the input bytes.
    fn leaf_hash(&mut self, bytes: impl AsRef<[u8]>) -> [u8; 32];

    /// Hash two inner nodes. This function *should* prepend a domain separator to the input.
    fn inner_hash(&mut self, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32];

    /// A trait describing how to build a merkle tree from a slice of bytes. The default implementation
    /// returns an RFC6962-style tree for ease of updating.
    fn build_merkle_tree(&mut self, byte_vecs: &[impl AsRef<[u8]>]) -> [u8; 32] {
        let length = byte_vecs.len();
        match length {
            0 => self.empty_root(),
            1 => self.leaf_hash(byte_vecs[0].as_ref()),
            _ => {
                let split = length.next_power_of_two() / 2;
                let left = self.build_merkle_tree(&byte_vecs[..split]);
                let right = self.build_merkle_tree(&byte_vecs[split..]);
                self.inner_hash(&left, &right)
            }
        }
    }
}

#[macro_export]
/// Implement the default `MerkleHasher` trait for a `SimpleHasher`, allowing it to build
/// domain-separated RC6962-style merkle trees.
macro_rules! impl_merkle_hasher(
	($name:ty) => {
		impl crate::crypto::simple_merkle::MerkleHasher for $name {
			fn empty_root(&mut self) -> [u8; 32] {
				<$name as SimpleHasher>::new().finalize()
			}

			fn leaf_hash(&mut self, bytes: impl AsRef<[u8]>) -> [u8; 32] {
				let mut hasher = <$name as SimpleHasher>::new();
				hasher.update(&crate::crypto::simple_merkle::DEFAULT_LEAF_DOMAIN_SEPARATOR);
				hasher.update(bytes.as_ref());
				hasher.finalize()
			}

			fn inner_hash(&mut self, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
				let mut hasher = <$name as SimpleHasher>::new();
				hasher.update(&crate::crypto::simple_merkle::DEFAULT_INTERNAL_DOMAIN_SEPARATOR);
				hasher.update(left);
				hasher.update(right);
				hasher.finalize()
			}
		}
	}
);
