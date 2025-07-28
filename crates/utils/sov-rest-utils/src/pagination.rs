use std::collections::HashMap;
use std::str::FromStr;

use serde::ser::SerializeMap;

const PAGE_SIZE_DEFAULT: u32 = 25;
const PAGE_SIZE_MAX: u32 = 100;

/// Query parameters that specify cursor-based pagination for a collection of
/// entities.
///
/// Read more about the tradeoffs of cursor-based VS offset-based pagination in
/// this great article: <https://slack.engineering/evolving-api-pagination-at-slack/>.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub struct Pagination<T> {
    /// The page size. No more than this number of items will be returned.
    pub size: u32,
    /// See [`PageSelection`].
    pub selection: PageSelection<T>,
}

impl<T> Default for Pagination<T> {
    fn default() -> Self {
        Self {
            size: PAGE_SIZE_DEFAULT,
            selection: Default::default(),
        }
    }
}

impl<T: serde::Serialize> serde::Serialize for Pagination<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_map(None)?;

        s.serialize_entry("page[size]", &self.size)?;
        match &self.selection {
            PageSelection::First => s.serialize_entry("page", "first")?,
            PageSelection::Last => s.serialize_entry("page", "last")?,
            PageSelection::Next { cursor } => {
                s.serialize_entry("page", "next")?;
                s.serialize_entry("page[cursor]", cursor)?;
            }
        }

        s.end()
    }
}

impl<'de, T: serde::Serialize + FromStr> serde::Deserialize<'de> for Pagination<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut map = HashMap::<String, String>::deserialize(deserializer)?;
        let size = map
            .remove("page[size]")
            // default
            .unwrap_or_else(|| PAGE_SIZE_DEFAULT.to_string())
            .parse::<u32>()
            .map_err(|_| {
                serde::de::Error::invalid_value(serde::de::Unexpected::Str("page[size]"), &"u32")
            })?;

        if size == 0 {
            return Err(serde::de::Error::custom(
                "page[size] must be greater than 0",
            ));
        } else if size > PAGE_SIZE_MAX {
            return Err(serde::de::Error::custom(format!(
                "page[size] must be less than or equal to {PAGE_SIZE_MAX}"
            )));
        }

        let selection = match map.remove("page").as_deref() {
            Some("next") => PageSelection::Next {
                cursor: map
                    .remove("page[cursor]")
                    .ok_or_else(|| serde::de::Error::missing_field("page[cursor]"))?
                    .parse::<T>()
                    .map_err(|_| {
                        serde::de::Error::invalid_value(
                            serde::de::Unexpected::Str("page[cursor]"),
                            &"T",
                        )
                    })?,
            },
            Some("first") => PageSelection::First,
            Some("last") => PageSelection::Last,
            _ => return Err(serde::de::Error::missing_field("page")),
        };

        Ok(Self { size, selection })
    }
}

/// What kind of page a client can request.
#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(proptest_derive::Arbitrary))]
pub enum PageSelection<T> {
    /// The next page of the collection. A request for the next page will
    /// require a cursor.
    Next {
        /// The page cursor, which specifies "where" the page starts within the
        /// collection.
        cursor: T,
    },
    /// The first page of the collection.
    #[default]
    First,
    /// The last page of the collection.
    Last,
}

/// The response of a paginated request.
///
/// Returns the items of the request as well as a
/// cursor for the next page if we're not at the end of the pagination.
#[derive(Debug, serde::Serialize)]
pub struct PaginatedResponse<T, C> {
    /// Items retireved by the request.
    pub items: Vec<T>,
    /// Cursor to use for the next page.
    ///
    /// If we're at the end of pagination this will be `None`.
    pub next_cursor: Option<C>,
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;
    use crate::axum_extractors::Query;
    use crate::test_utils::uri_with_query_params;

    #[test_strategy::proptest]
    fn serialization_roundtrip_equality(sorting: Pagination<String>) {
        if (1..=PAGE_SIZE_MAX).contains(&sorting.size) {
            let serialized = serde_urlencoded::to_string(&sorting)?;
            let deserialized: Pagination<String> = serde_urlencoded::from_str(&serialized)?;
            assert_eq!(sorting, deserialized);
        }
    }

    fn try_deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<Pagination<String>> {
        let uri = uri_with_query_params(query_params);
        let validated_query = Query::<Pagination<String>>::try_from_uri(&uri)
            // The query rejection type is not a valid error, so we replace it with a dummy error type.
            .map_err(|_| anyhow!("error"))?;

        Ok(validated_query.0)
    }

    #[test]
    fn ok_cases() {
        try_deserialize(&[
            ("page[size]", "10"),
            ("page[cursor]", "foobar"),
            ("page", "next"),
        ])
        .unwrap();

        try_deserialize(&[("page", "first")]).unwrap();
        try_deserialize(&[("page", "last")]).unwrap();
        try_deserialize(&[("page", "last"), ("page[size]", "10")]).unwrap();
    }

    #[test]
    fn bad_page_size() {
        try_deserialize(&[("page[size]", "-10")]).unwrap_err();
        try_deserialize(&[("page[size]", "0")]).unwrap_err();
        try_deserialize(&[("page[size]", "100000")]).unwrap_err();
    }

    #[test]
    fn cursor_with_next_is_mandatory() {
        try_deserialize(&[("page", "next"), ("page[cursor]", "foo")]).unwrap();
        try_deserialize(&[("page", "next")]).unwrap_err();
    }
}
