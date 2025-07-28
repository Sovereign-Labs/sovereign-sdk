//! Query string parsing and validation for sorting options.

use std::fmt::Display;
use std::str::FromStr;

/// Single-column sorting options.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub struct Sorting<T> {
    /// The field(s) to sort by.
    pub by: T,

    /// The sorting order.
    pub order: SortingOrder,
}

impl<'a, T> serde::Deserialize<'a> for Sorting<T>
where
    T: FromStr,
    T::Err: Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Sorting<T>, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let string = String::deserialize(deserializer).map_err(|e| {
            serde::de::Error::custom(format!("failed to deserialize sorting string: {e}"))
        })?;

        let mut chars = string.chars();
        let (order, sort_by_str) = match chars.next() {
            Some('-') => (SortingOrder::Descending, chars.as_str()),
            Some('+') => (SortingOrder::Ascending, chars.as_str()),
            Some(c) => {
                return Err(serde::de::Error::custom(format!(
                    "invalid sorting order character, must be either '+' or '-': {c}"
                )));
            }
            None => {
                return Err(serde::de::Error::custom(
                    "empty sorting value is not allowed",
                ));
            }
        };

        // Restrict allowed characters to alphanumeric, hyphen, and underscore.
        // If we don't do this, weird edge cases could come up.
        if !sort_by_str
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(serde::de::Error::custom(
                "the sort-by field can only contain alphanumeric characters, hyphens, and underscores",
            ));
        }

        let sort_by = T::from_str(sort_by_str).map_err(|e| {
            serde::de::Error::custom(format!("failed to parse sorting string: {e}"))
        })?;

        Ok(Sorting { by: sort_by, order })
    }
}

impl<T> serde::Serialize for Sorting<T>
where
    T: ToString,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let sign = match self.order {
            SortingOrder::Ascending => "+",
            SortingOrder::Descending => "-",
        };
        format!("{}{}", sign, self.by.to_string()).serialize(serializer)
    }
}

/// The sorting order of something.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub enum SortingOrder {
    /// Ascending order.
    Ascending,
    /// Descending order.
    Descending,
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use proptest_derive::Arbitrary;

    use super::*;
    use crate::test_utils::{
        test_serialization_roundtrip_equality_urlencoded, uri_with_query_params,
    };

    #[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize, Arbitrary)]
    struct SortingQuery {
        // Testing with signed integers is better than strings because we can
        // catch bugs related to handling of '-'.
        sort: Sorting<i32>,
    }

    #[test_strategy::proptest]
    fn serialization_roundtrip(sorting: SortingQuery) {
        test_serialization_roundtrip_equality_urlencoded(sorting);
    }

    fn try_deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<SortingQuery> {
        let uri = uri_with_query_params(query_params);
        Ok(Query::<SortingQuery>::try_from_uri(&uri)
            .map_err(|e| {
                anyhow::anyhow!("failed to parse sorting query string: {}", e.to_string())
            })?
            .0)
    }

    #[test]
    fn ok_cases() {
        try_deserialize(&[("sort", "+100")]).unwrap();
        try_deserialize(&[("sort", "-100")]).unwrap();
        try_deserialize(&[("sort", "+100")]).unwrap();
        try_deserialize(&[("sort", "--100")]).unwrap();
        try_deserialize(&[("sort", "+0")]).unwrap();
    }

    #[test]
    fn disallowed_characters() {
        try_deserialize(&[("sort", "+foo")]).unwrap_err();
        try_deserialize(&[("sort", "-a")]).unwrap_err();
        try_deserialize(&[("sort", "-#100")]).unwrap_err();
        try_deserialize(&[("sort", "-1.2")]).unwrap_err();
    }

    #[test]
    fn empty_value() {
        try_deserialize(&[("sort", "+")]).unwrap_err();
        try_deserialize(&[("sort", "-")]).unwrap_err();
        try_deserialize(&[("sort", "$")]).unwrap_err();
        try_deserialize(&[("sort", "")]).unwrap_err();
    }
}
