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
        let string = String::deserialize(deserializer)?;

        let mut chars = string.chars();
        let (order, sort_by) = match chars.next() {
            Some('-') => (SortingOrder::Descending, chars.as_str()),
            Some('+') => (SortingOrder::Ascending, chars.as_str()),
            Some(_) => (SortingOrder::Ascending, string.as_str()),
            None => {
                return Err(serde::de::Error::custom(
                    "Empty sorting value is not allowed",
                ))
            }
        };
        if sort_by.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return Err(serde::de::Error::custom(
                "The sort-by field can only contain alphanumeric characters",
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Arbitrary)]
pub enum SortingOrder {
    Ascending,
    Descending,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils;

    #[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
    struct SortingQuery {
        // Testing with signed integers is better than strings because we can
        // catch bugs related to handling of '-'.
        sort: Sorting<i32>,
    }

    async fn deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<SortingQuery> {
        let deserialized: SortingQuery = utils::serialize_query_params(query_params).await?;

        // As an extra test we get for free, we serialize and deserialize the
        // query again. When serialized they may not be equal, but semantic
        // comparison should be equal.
        let deserialized_round_trip = serde_json::from_str(&serde_json::to_string(&deserialized)?)?;
        if deserialized != deserialized_round_trip {
            anyhow::bail!("Round trip failed");
        }

        Ok(deserialized)
    }

    #[tokio::test]
    async fn ok_cases() {
        deserialize(&[("sort", "foo")]).await.unwrap_err();
        deserialize(&[("sort", "a")]).await.unwrap_err();
        deserialize(&[("sort", "100")]).await.unwrap();
        deserialize(&[("sort", "+100")]).await.unwrap();
        deserialize(&[("sort", "-100")]).await.unwrap();
    }

    #[tokio::test]
    async fn disallowed_characters() {
        deserialize(&[("sort", "_100")]).await.unwrap_err();
        deserialize(&[("sort", "-_100")]).await.unwrap_err();
        deserialize(&[("sort", "+-100")]).await.unwrap_err();
    }

    #[tokio::test]
    async fn empty_value() {
        deserialize(&[("sort", "+")]).await.unwrap_err();
        deserialize(&[("sort", "-")]).await.unwrap_err();
    }
}
