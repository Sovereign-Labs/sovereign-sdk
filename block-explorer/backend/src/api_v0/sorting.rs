//! Query string parsing and validation for sorting settings.

use std::fmt::Display;
use std::str::FromStr;

use super::extractors::QueryValidation;

/// A specification on how to sort a list of items.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
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
        let string = String::deserialize(deserializer)?;

        let mut chars = string.chars();
        let (order, sort_by) = match chars.next() {
            Some('-') => (SortingOrder::Descending, chars.as_str()),
            Some('+') => (SortingOrder::Ascending, chars.as_str()),
            Some(_) => (SortingOrder::Ascending, string.as_str()),
            None => {
                return Err(serde::de::Error::custom(
                    "empty sorting value is not allowed",
                ))
            }
        };
        if sort_by.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return Err(serde::de::Error::custom(
                "the sort-by field can only contain alphanumeric characters",
            ));
        }

        let by = T::from_str(sort_by).map_err(serde::de::Error::custom)?;
        Ok(Sorting { by, order })
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

impl<T> Sorting<T> {
    pub fn map_to_string<F>(&self, f: F) -> Sorting<&'static str>
    where
        F: Fn(&T) -> &'static str,
    {
        Sorting {
            by: f(&self.by),
            order: self.order,
        }
    }
}

impl<T> QueryValidation for Sorting<T> {
    fn validate(&self) -> anyhow::Result<()> {
        // No validation required.
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum SortingOrder {
    Ascending,
    Descending,
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use proptest::proptest;
    use proptest_derive::Arbitrary;

    use super::*;
    use crate::utils::{test_serialization_roundtrip_equality_urlencoded, uri_with_query_params};

    #[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize, Arbitrary)]
    struct SortingQueryU32 {
        sort: Sorting<u32>,
    }

    proptest! {
        #[test]
        fn serialization_roundtrip(sorting: SortingQueryU32) {
            test_serialization_roundtrip_equality_urlencoded(sorting);
        }
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
    struct SortingQuery {
        // Testing with signed integers is better than strings because we can
        // catch bugs related to handling of '-'. However, not all valid struct values can be
        // encoded because of the minus sign, so it can't be used with `proptest`.
        sort: Sorting<i32>,
    }

    fn try_deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<SortingQuery> {
        let uri = uri_with_query_params(query_params);
        Ok(Query::<SortingQuery>::try_from_uri(&uri)?.0)
    }

    #[test]
    fn ok_cases() {
        try_deserialize(&[("sort", "foo")]).unwrap_err();
        try_deserialize(&[("sort", "a")]).unwrap_err();
        try_deserialize(&[("sort", "100")]).unwrap();
        try_deserialize(&[("sort", "+100")]).unwrap();
        try_deserialize(&[("sort", "-100")]).unwrap();
    }

    #[test]
    fn disallowed_characters() {
        try_deserialize(&[("sort", "_100")]).unwrap_err();
        try_deserialize(&[("sort", "-_100")]).unwrap_err();
        try_deserialize(&[("sort", "+-100")]).unwrap_err();
    }

    #[test]
    fn empty_value() {
        try_deserialize(&[("sort", "+")]).unwrap_err();
        try_deserialize(&[("sort", "-")]).unwrap_err();
    }
}
