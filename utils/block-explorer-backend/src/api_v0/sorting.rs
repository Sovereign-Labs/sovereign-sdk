//! Query string parsing and validation for sorting settings.

use std::fmt::Display;
use std::str::FromStr;

use arbitrary::Arbitrary;

#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
pub struct Sorting<T> {
    pub by: T,
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
        let s = String::deserialize(deserializer)?;
        let mut chars = s.chars();
        let order = match chars.next() {
            Some('-') => SortingOrder::Descending,
            _ => SortingOrder::Ascending,
        };
        let by = T::from_str(chars.as_str()).map_err(serde::de::Error::custom)?;
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
        format!("{}{}", self.by.to_string(), sign).serialize(serializer)
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Arbitrary)]
pub enum SortingOrder {
    Ascending,
    Descending,
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use axum::http::Uri;

    use super::*;

    #[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
    struct SortingQuery<T> {
        sort: Sorting<T>,
    }

    fn deserialize(query: &str) -> anyhow::Result<SortingQuery<String>> {
        let uri = query.parse().expect("Failed to parse URI");
        let deserialized = Query::try_from_uri(uri)?.0;

        // As an extra test we get for free, we serialize and deserialize the
        // query again. When serialized they may not be equal, but semantic
        // comparison should be equal.
        let deserialized_round_trip = serde_json::from_str(&serde_json::to_string(&deserialized)?)?;
        if deserialized != deserialized_round_trip {
            anyhow::bail!("Round trip failed");
        }

        Ok(Query::try_from_uri(uri)?.0)
    }

    #[test]
    fn deserialize() {
        deserialize("http://example.com?sort=id").unwrap();
        deserialize("http://example.com?sort=+id").unwrap();
        deserialize("http://example.com?sort=-id").unwrap();
    }

    #[test]
    fn disallowed_characters() {
        deserialize("http://example.com?sort=_age").unwrap_err();
        deserialize("http://example.com?sort=-_age").unwrap_err();
        deserialize("http://example.com?sort=+-age").unwrap_err();
    }

    #[test]
    fn empty_value() {
        deserialize("http://example.com?sort").unwrap_err();
        deserialize("http://example.com?sort=").unwrap_err();
        deserialize("http://example.com?sort=+").unwrap_err();
        deserialize("http://example.com?sort=-").unwrap_err();
    }
}
