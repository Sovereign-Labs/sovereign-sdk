use arbitrary::Arbitrary;

#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Pagination<T> {
    #[serde(
        rename = "page[size]",
        default = "default_pagination_size",
        // Not necessary, but it makes pagination links shorter.
        skip_serializing_if = "is_default_pagination_size"
    )]
    pub size: u32,
    #[serde(
        rename = "page[selection]",
        default,
        // Once again, not strictly necessary.
        skip_serializing_if = "is_default"
    )]
    pub selection: PageSelection,
    #[serde(rename = "page[cursor]", default = "Option::default")]
    pub cursor: Option<T>,
}

impl<T> Pagination<T> {
    pub fn validate(&self) -> anyhow::Result<()> {
        // Bad page sizes.
        if self.size == 0 || self.size > max_pagination_size() {
            anyhow::bail!("Page size must be between 1 and {}", max_pagination_size());
        }

        // Cursor is required for next/prev, but it doesn't make sense for
        // first/last.
        match (&self.selection, self.cursor.is_some()) {
            (PageSelection::Next | PageSelection::Prev, false) => {
                anyhow::bail!("Cursor is required for next/prev")
            }
            (PageSelection::First | PageSelection::Last, true) => {
                anyhow::bail!("Cursor is required for next/prev")
            }
            _ => (),
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize, Arbitrary)]
#[serde(rename_all = "camelCase")]
pub enum PageSelection {
    Next,
    Prev,
    First,
    Last,
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

pub fn is_default_pagination_size(size: &u32) -> bool {
    *size == default_pagination_size()
}

pub fn is_default<T: PartialEq + Default>(item: &T) -> bool {
    item == &T::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils;

    async fn deserialize(query_params: &[(&str, &str)]) -> anyhow::Result<Pagination<String>> {
        let deserialized: Pagination<String> = utils::serialize_query_params(query_params).await?;

        // Important!
        deserialized.validate()?;

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
        deserialize(&[
            ("page[size]", "10"),
            ("page[cursor]", "foobar"),
            ("page[selection]", "next"),
        ])
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn bad_page_size() {
        deserialize(&[("page[size]", "-10")]).await.unwrap_err();
        deserialize(&[("page[size]", "0")]).await.unwrap_err();
        deserialize(&[("page[size]", "100000")]).await.unwrap_err();
    }

    #[tokio::test]
    async fn cursor() {
        deserialize(&[("page[selection]", "next"), ("page[cursor]", "foo")])
            .await
            .unwrap();
        deserialize(&[("page[selection]", "prev"), ("page[cursor]", "foo")])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cursor_with_first_and_last_pages_not_ok() {
        deserialize(&[("page[selection]", "first"), ("page[cursor]", "foo")])
            .await
            .unwrap_err();
        deserialize(&[("page[selection]", "last"), ("page[cursor]", "foo")])
            .await
            .unwrap_err();
    }
}
