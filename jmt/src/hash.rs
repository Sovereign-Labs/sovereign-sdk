// Adapted from aptos-crypto
// SPDX-License-Identifier: Apache-2.0
// Modified to make HashValue generic over output size
use core::{fmt, str::FromStr};
#[cfg(any(test, feature = "fuzzing"))]
use proptest::strategy::{BoxedStrategy, Strategy};
#[cfg(any(test, feature = "fuzzing"))]
use proptest::{
    collection::vec,
    prelude::{any, Arbitrary},
};
#[cfg(any(test, feature = "fuzzing"))]
use rand::Rng;
use serde::{de, ser};
#[cfg(any(test, feature = "fuzzing"))]
use tiny_keccak::{Hasher, Sha3};

/// Output value of a function. Intentionally opaque for safety and modularity.
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct HashOutput<const N: usize> {
    hash: [u8; N],
}

#[cfg(any(test, feature = "fuzzing"))]
impl<const N: usize> Arbitrary for HashOutput<N> {
    type Parameters = ();

    // TODO(preston-evans98): revert to more efficient impl below once proptest supports const generics
    // fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
    //     any::<[u8; N]>().prop_map(|x| HashValue { hash: x }).boxed()
    // }
    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        vec(any::<u8>(), N)
            .prop_map(|bytes| {
                let mut out = [0u8; N];
                out.copy_from_slice(bytes.as_ref());
                HashOutput::new(out)
            })
            .boxed()
    }

    type Strategy = BoxedStrategy<Self>;
}

impl<const N: usize> HashOutput<N> {
    /// The length of the hash in bytes.
    pub const LENGTH: usize = N;
    /// The length of the hash in bits.
    pub const LENGTH_IN_BITS: usize = Self::LENGTH * 8;
    /// The longest path allowed in a merkle tree with this hash size.
    /// Equal to the length of the hash in nibbles.
    pub const ROOT_NIBBLE_HEIGHT: usize = Self::LENGTH * 2;

    /// Create a new [`HashValue`] from a byte array.
    pub const fn new(hash: [u8; N]) -> Self {
        HashOutput { hash }
    }

    /// Create from a slice (e.g. retrieved from storage).
    pub fn from_slice<T: AsRef<[u8]>>(bytes: T) -> Result<Self, HashValueParseError> {
        <[u8; N]>::try_from(bytes.as_ref())
            .map_err(|_| HashValueParseError)
            .map(Self::new)
    }

    /// Dumps into a vector.
    pub fn to_vec(&self) -> Vec<u8> {
        self.hash.to_vec()
    }

    /// Creates a zero-initialized instance.
    pub const fn zero() -> Self {
        HashOutput { hash: [0; N] }
    }

    /// Create a cryptographically random instance.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn random() -> Self {
        use rand::rngs::OsRng;

        let mut rng = OsRng;
        let hash: [u8; N] = rng.gen();
        HashOutput { hash }
    }

    /// Creates a random instance with given rng. Useful in unit tests.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn random_with_rng<R: rand::Rng>(rng: &mut R) -> Self {
        let hash: [u8; N] = rng.gen();
        HashOutput { hash }
    }

    /// Convenience function that computes a `HashValue` internally equal to
    /// the sha3_256 of a byte buffer. It will handle hasher creation, data
    /// feeding and finalization.
    ///
    /// Note this will not result in the `<T as CryptoHash>::hash()` for any
    /// reasonable struct T, as this computes a sha3 without any ornaments.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn sha3_256_of(buffer: &[u8]) -> Self {
        let mut sha3 = Sha3::v256();
        sha3.update(buffer);
        HashOutput::from_keccak(sha3)
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub fn from_iter_sha3<'a, I>(buffers: I) -> Self
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut sha3 = Sha3::v256();
        for buffer in buffers {
            sha3.update(buffer);
        }
        HashOutput::from_keccak(sha3)
    }

    #[cfg(any(test, feature = "fuzzing"))]
    fn as_ref_mut(&mut self) -> &mut [u8] {
        &mut self.hash[..]
    }

    #[cfg(any(test, feature = "fuzzing"))]
    fn from_keccak(state: Sha3) -> Self {
        let mut hash = Self::zero();
        state.finalize(hash.as_ref_mut());
        hash
    }

    /// Returns the `index`-th bit in the bytes.
    pub fn bit(&self, index: usize) -> bool {
        debug_assert!(index < Self::LENGTH_IN_BITS); // assumed precondition
        let pos = index / 8;
        let bit = 7 - index % 8;
        (self.hash[pos] >> bit) & 1 != 0
    }

    /// Returns the `index`-th nibble in the bytes.
    pub fn nibble(&self, index: usize) -> u8 {
        debug_assert!(index < Self::LENGTH * 2); // assumed precondition
        let pos = index / 2;
        let shift = if index % 2 == 0 { 4 } else { 0 };
        (self.hash[pos] >> shift) & 0x0F
    }

    /// Returns a `HashValueBitIterator` over all the bits that represent this `HashValue`.
    pub fn iter_bits(&self) -> HashValueBitIterator<'_, N> {
        HashValueBitIterator::new(self)
    }

    /// Constructs a `HashValue` from an iterator of bits.
    pub fn from_bit_iter(
        iter: impl ExactSizeIterator<Item = bool>,
    ) -> Result<Self, HashValueParseError> {
        if iter.len() != Self::LENGTH_IN_BITS {
            return Err(HashValueParseError);
        }

        let mut buf = [0; N];
        for (i, bit) in iter.enumerate() {
            if bit {
                buf[i / 8] |= 1 << (7 - i % 8);
            }
        }
        Ok(Self::new(buf))
    }

    /// Returns the length of common prefix of `self` and `other` in bits.
    pub fn common_prefix_bits_len(&self, other: HashOutput<N>) -> usize {
        self.iter_bits()
            .zip(other.iter_bits())
            .take_while(|(x, y)| x == y)
            .count()
    }

    /// Full hex representation of a given hash value.
    pub fn to_hex(&self) -> String {
        format!("{self:x}")
    }

    /// Full hex representation of a given hash value with `0x` prefix.
    pub fn to_hex_literal(&self) -> String {
        format!("{self:#x}")
    }

    /// Parse a given hex string to a hash value.
    pub fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, HashValueParseError> {
        <[u8; N]>::my_from_hex(hex)
            .map_err(|_| HashValueParseError)
            .map(Self::new)
    }

    /// Create a hash value whose contents are just the given integer. Useful for
    /// generating basic mock hash values.
    ///
    /// Ex: HashValue::from_u64(0x1234) => HashValue([0, .., 0, 0x12, 0x34])
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn from_u64(v: u64) -> Self {
        let mut hash = [0u8; N];
        let bytes = v.to_be_bytes();
        hash[N - bytes.len()..].copy_from_slice(&bytes[..]);
        Self::new(hash)
    }
}

impl<const N: usize> ser::Serialize for HashOutput<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_hex())
        } else {
            // In order to preserve the Serde data model and help analysis tools,
            // make sure to wrap our value in a container with the same name
            // as the original type.
            // #[derive(Serialize)]
            // #[serde(rename = "HashValue")]
            // struct Value<'a> {
            //     hash: &'a [u8],
            // }
            // Value { hash: &self.hash }.serialize(serializer)
            self.hash.serialize(serializer)
        }
    }
}

pub trait MyFromHex: Sized {
    fn my_from_hex(arg: impl AsRef<[u8]>) -> Result<Self, hex::FromHexError>;
}

impl<const N: usize> MyFromHex for [u8; N] {
    fn my_from_hex(arg: impl AsRef<[u8]>) -> Result<Self, hex::FromHexError> {
        let mut out = [0u8; N];
        hex::decode_to_slice(arg, &mut out as &mut [u8])?;
        Ok(out)
    }
}

impl<'de, const N: usize> de::Deserialize<'de> for HashOutput<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let encoded_hash = <String>::deserialize(deserializer)?;
            HashOutput::from_hex(encoded_hash.as_str())
                .map_err(<D::Error as ::serde::de::Error>::custom)
        } else {
            // // See comment in serialize.
            // #[derive(Deserialize)]
            // #[serde(rename = "HashValue")]
            // struct Value {
            //     hash: [u8; HashValue::<N>::LENGTH],
            // }

            // let value = Value::deserialize(deserializer)
            //     .map_err(<D::Error as ::serde::de::Error>::custom)?;
            let mut out = [0u8; N];
            let value = <&[u8]>::deserialize(deserializer)?;
            out.copy_from_slice(value);
            Ok(Self::new(out))
            // Ok(Self::new(value.hash))
        }
    }
}

impl<const N: usize> Default for HashOutput<N> {
    fn default() -> Self {
        HashOutput::zero()
    }
}

impl<const N: usize> AsRef<[u8; N]> for HashOutput<N> {
    fn as_ref(&self) -> &[u8; N] {
        &self.hash
    }
}

impl<const N: usize> std::ops::Deref for HashOutput<N> {
    type Target = [u8; N];

    fn deref(&self) -> &Self::Target {
        &self.hash
    }
}

impl<const N: usize> std::ops::Index<usize> for HashOutput<N> {
    type Output = u8;

    fn index(&self, s: usize) -> &u8 {
        self.hash.index(s)
    }
}

impl<const N: usize> fmt::Binary for HashOutput<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.hash {
            write!(f, "{byte:08b}")?;
        }
        Ok(())
    }
}

impl<const N: usize> fmt::LowerHex for HashOutput<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "0x")?;
        }
        for byte in &self.hash {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<const N: usize> fmt::Debug for HashOutput<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HashValue(")?;
        <Self as fmt::LowerHex>::fmt(self, f)?;
        write!(f, ")")?;
        Ok(())
    }
}

/// Will print shortened (4 bytes) hash
impl<const N: usize> fmt::Display for HashOutput<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for byte in self.hash.iter().take(4) {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

// TODO(preston-evans98): consider adding back
// impl From<HashValue> for Bytes {
//     fn from(value: HashValue) -> Bytes {
//         Bytes::copy_from_slice(value.hash.as_ref())
//     }
// }

impl<const N: usize> FromStr for HashOutput<N> {
    type Err = HashValueParseError;

    fn from_str(s: &str) -> Result<Self, HashValueParseError> {
        HashOutput::from_hex(s)
    }
}

/// Parse error when attempting to construct a HashValue
#[derive(Clone, Copy, Debug)]
pub struct HashValueParseError;

impl fmt::Display for HashValueParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "unable to parse HashValue")
    }
}

impl std::error::Error for HashValueParseError {}

/// An iterator over `HashValue` that generates one bit for each iteration.
pub struct HashValueBitIterator<'a, const N: usize> {
    /// The reference to the bytes that represent the `HashValue`.
    hash_bytes: &'a [u8],
    pos: std::ops::Range<usize>,
    // invariant hash_bytes.len() == HashValue::LENGTH;
    // invariant pos.end == hash_bytes.len() * 8;
}

impl<'a, const N: usize> HashValueBitIterator<'a, N> {
    /// Constructs a new `HashValueBitIterator` using given `HashValue`.
    fn new(hash_value: &'a HashOutput<N>) -> Self {
        HashValueBitIterator {
            hash_bytes: hash_value.as_ref(),
            pos: (0..HashOutput::<N>::LENGTH_IN_BITS),
        }
    }

    /// Returns the `index`-th bit in the bytes.
    fn get_bit(&self, index: usize) -> bool {
        debug_assert_eq!(self.hash_bytes.len(), N); // invariant
        debug_assert!(index <= self.hash_bytes.len() * 8); // assumed precondition
        let pos = index / 8;
        let bit = 7 - index % 8;
        (self.hash_bytes[pos] >> bit) & 1 != 0
    }
}

impl<'a, const N: usize> std::iter::Iterator for HashValueBitIterator<'a, N> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        self.pos.next().map(|x| self.get_bit(x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.pos.size_hint()
    }
}

impl<'a, const N: usize> std::iter::DoubleEndedIterator for HashValueBitIterator<'a, N> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.pos.next_back().map(|x| self.get_bit(x))
    }
}

impl<'a, const N: usize> std::iter::ExactSizeIterator for HashValueBitIterator<'a, N> {}

pub trait TreeHash<const N: usize>: std::fmt::Debug + Send + Sync + std::cmp::PartialEq {
    type Hasher: CryptoHasher<N>;
    const SPARSE_MERKLE_PLACEHOLDER_HASH: HashOutput<N>;
    fn hash(data: impl AsRef<[u8]>) -> HashOutput<N> {
        Self::Hasher::new().update(data.as_ref()).finalize()
    }

    fn hasher() -> Self::Hasher {
        Self::Hasher::new()
    }
}

pub trait CryptoHasher<const N: usize> {
    fn new() -> Self;
    fn update(self, data: &[u8]) -> Self;
    fn finalize(self) -> HashOutput<N>;
}
