use std::collections::HashSet;
use std::fmt::{Debug, Display};

use url::Url;

/// A newtype wrapper around [`Vec<u8>`] which is serialized as a
/// 0x-prefixed hex string.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct HexString(pub Vec<u8>);

impl AsRef<Vec<u8>> for HexString {
    fn as_ref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl Display for HexString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl serde::Serialize for HexString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'a> serde::Deserialize<'a> for HexString {
    fn deserialize<D>(deserializer: D) -> Result<HexString, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let string = String::deserialize(deserializer)?;
        // We ignore the 0x prefix if it exists.
        let s = string.strip_prefix("0x").unwrap_or(&string);

        hex::decode(s)
            .map_err(|e| anyhow::anyhow!("Failed to decode hex: {}", e))
            .map(HexString)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct CommaSeparatedStrings(pub HashSet<String>);

impl CommaSeparatedStrings {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl serde::Serialize for CommaSeparatedStrings {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = String::new();
        for (i, string) in self.0.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(string);
        }
        s.serialize(serializer)
    }
}

impl<'a> serde::Deserialize<'a> for CommaSeparatedStrings {
    fn deserialize<D>(deserializer: D) -> Result<CommaSeparatedStrings, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let string = String::deserialize(deserializer)?;
        let hashset = string
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();

        Ok(Self(hashset))
    }
}

#[cfg(test)]
pub fn uri_with_query_params<T>(params: T) -> axum::http::Uri
where
    T: serde::Serialize,
{
    use std::str::FromStr;

    use axum::http::Uri;

    // See
    // <https://github.com/nox/serde_urlencoded/blob/master/tests/test_serialize.rs>
    // for some examples.
    let s = format!(
        "http://example.com?{}",
        serde_urlencoded::to_string(params).unwrap()
    );
    Uri::from_str(&s).expect("Can't create URI from string")
}

pub fn upsert_query_pair_in_url(url: &mut Url, key: &str, value: &str) {
    let original_query_pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();

    let mut query_pairs = url.query_pairs_mut();
    query_pairs.clear();
    let mut modified = false;
    for (k, v) in original_query_pairs {
        if k == key {
            query_pairs.append_pair(&key, &value);
            modified = true;
        } else {
            query_pairs.append_pair(&k, &v);
        }
    }
    if !modified {
        query_pairs.append_pair(&key, &value);
    }

    query_pairs.finish();
}

#[cfg(test)]
pub fn test_serialization_roundtrip_equality_urlencoded<T>(item: T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let serialized = serde_urlencoded::to_string(&item).unwrap();
    let deserialized: T = serde_urlencoded::from_str(&serialized).unwrap();
    assert_eq!(item, deserialized);
}

#[cfg(test)]
pub fn test_serialization_roundtrip_equality_json<T>(item: T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let serialized = serde_json::to_string(&item).unwrap();
    let deserialized: T = serde_json::from_str(&serialized).unwrap();
    assert_eq!(item, deserialized);
}

#[cfg(test)]
mod tests {

    use proptest::proptest;

    use super::*;

    proptest! {
        #[test]
        fn hex_string_serialization_roundtrip(item: HexString) {
            test_serialization_roundtrip_equality_json(item);
        }

        #[test]
        fn comma_separated_strings_serialization_roundtrip(numbers: Vec<i32>) {
            let item = CommaSeparatedStrings(numbers.into_iter().map(|i| i.to_string()).collect());
            test_serialization_roundtrip_equality_json(item);
        }

        // Ideally we'd also test with types other than strings. E.g. integers?
        #[test]
        fn any_query_param_can_be_serialized(key: String, value: String) {
            // As long as it doesn't crash, we're good and the test succeeds.
            uri_with_query_params(&[(key, value)]);
        }
    }
}
