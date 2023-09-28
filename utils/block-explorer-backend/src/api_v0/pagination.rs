use arbitrary::Arbitrary;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Pagination<T> {
    #[serde(
        rename = "page[size]",
        deserialize_with = "crate::utils::deserialize_from_str",
        default = "default_pagination_size",
        skip_serializing_if = "is_default_pagination_size"
    )]
    pub size: u32,
    #[serde(rename = "page[selection]", default)]
    pub selection: PageSelection,
    #[serde(rename = "page[cursor]", default = "Option::default")]
    pub cursor: Option<T>,
}

impl<T> Pagination<T> {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.size == 0 || self.size > max_pagination_size() {
            anyhow::bail!("Page size must be between 1 and {}", max_pagination_size());
        }

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Arbitrary)]
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

pub const fn is_default_pagination_size(size: &u32) -> bool {
    *size == default_pagination_size()
}

#[cfg(test)]
mod tests {
    use super::*;
}
