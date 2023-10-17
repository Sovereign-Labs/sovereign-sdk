use serde_with::serde_as;
use url::Url;

use super::api_utils::PaginationLinks;
use super::extractors::QueryValidation;
use crate::utils::upsert_query_pair_in_url;

/// Query parameters that specify cursor-based pagination for a collection of
/// entities.
///
/// Read more about the tradeoffs of cursor-based VS offset-based pagination in
/// this great article: <https://slack.engineering/evolving-api-pagination-at-slack/>.
// Workaround for Serde bug: <https://docs.rs/serde_qs/0.12.0/serde_qs/index.html#flatten-workaround>
#[serde_as]
#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Pagination<T> {
    /// The maximum allowed number of entities to return at once.
    #[serde(rename = "page[size]", default = "default_pagination_size")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub size: u32,
    /// See [`PageSelection`].
    #[serde(rename = "page[selection]", default)]
    pub selection: PageSelection,
    /// The page cursor, which specifies "where" the page starts within the
    /// collection.
    ///
    /// The cursor is incompatible with first/last pages and optional for
    /// next/prev. If unspecified for next/prev, the first/last pages are
    /// returned instead.
    #[serde(rename = "page[cursor]", default = "Option::default")]
    pub cursor: Option<T>,
}

impl<T> Pagination<T> {
    pub fn links(&self, url: &str, new_cursor_value: &str) -> PaginationLinks {
        PaginationLinks {
            first: update_url_with_pagination(url, PageSelection::First, new_cursor_value),
            next: update_url_with_pagination(url, PageSelection::Next, new_cursor_value),
            prev: update_url_with_pagination(url, PageSelection::Prev, new_cursor_value),
            last: update_url_with_pagination(url, PageSelection::Last, new_cursor_value),
        }
    }
}

fn update_url_with_pagination(
    url: &str,
    page_selection: PageSelection,
    new_cursor: &str,
) -> String {
    let mut url = Url::parse(url).unwrap();
    upsert_query_pair_in_url(&mut url, "page[selection]", &page_selection.to_string());
    if page_selection.is_compatible_with_cursor() {
        upsert_query_pair_in_url(&mut url, "page[cursor]", new_cursor);
    }
    url.to_string()
}

impl<T> QueryValidation for Pagination<T> {
    fn validate(&self) -> anyhow::Result<()> {
        // Bad page sizes.
        if self.size == 0 || self.size > max_pagination_size() {
            anyhow::bail!("Page size must be between 1 and {}", max_pagination_size());
        }

        // Cursor is incompatible with first/last.
        if !self.selection.is_compatible_with_cursor() && self.cursor.is_some() {
            anyhow::bail!("Cursor is not compatible with first/last pages");
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub enum PageSelection {
    Next,
    Prev,
    First,
    Last,
}

impl PageSelection {
    pub fn is_compatible_with_cursor(&self) -> bool {
        matches!(self, PageSelection::Next | PageSelection::Prev)
    }
}

impl ToString for PageSelection {
    fn to_string(&self) -> String {
        serde_json::to_value(self)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Default for PageSelection {
    fn default() -> Self {
        PageSelection::Next
    }
}

pub const fn max_pagination_size() -> u32 {
    250
}

pub const fn default_pagination_size() -> u32 {
    25
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use proptest::proptest;

    use super::*;
    use crate::api_v0::extractors::ValidatedQuery;
    use crate::utils::uri_with_query_params;

    proptest! {
        #[test]
        fn serialization_roundtrip_equality(sorting: Pagination<String>) {
            let serialized = serde_urlencoded::to_string(&sorting)?;
            let deserialized: Pagination<String> = serde_urlencoded::from_str(&serialized)?;
            assert_eq!(sorting, deserialized);
        }
    }

    #[test]
    fn page_selection_to_string() {
        assert_eq!(PageSelection::First.to_string(), "first");
    }

    fn try_deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<Pagination<String>> {
        let uri = uri_with_query_params(query_params);
        let validated_query = ValidatedQuery::<Pagination<String>>::try_from_uri(&uri)
            // The query rejection type is not a valid error, so we replace it with a dummy error type.
            .map_err(|_| anyhow!("error"))?;

        Ok(validated_query.0)
    }

    #[test]
    fn ok_cases() {
        try_deserialize(&[
            ("page[size]", "10"),
            ("page[cursor]", "foobar"),
            ("page[selection]", "next"),
        ])
        .unwrap();
    }

    #[test]
    fn bad_page_size() {
        try_deserialize(&[("page[size]", "-10")]).unwrap_err();
        try_deserialize(&[("page[size]", "0")]).unwrap_err();
        try_deserialize(&[("page[size]", "100000")]).unwrap_err();
    }

    #[test]
    fn cursor_with_next_and_prev_is_optional() {
        try_deserialize(&[("page[selection]", "next"), ("page[cursor]", "foo")]).unwrap();
        try_deserialize(&[("page[selection]", "prev"), ("page[cursor]", "foo")]).unwrap();

        try_deserialize(&[("page[selection]", "next")]).unwrap();
        try_deserialize(&[("page[selection]", "prev")]).unwrap();
    }

    #[test]
    fn cursor_with_first_and_last_not_ok() {
        try_deserialize(&[("page[selection]", "first"), ("page[cursor]", "foo")]).unwrap_err();
        try_deserialize(&[("page[selection]", "last"), ("page[cursor]", "foo")]).unwrap_err();
    }
}
