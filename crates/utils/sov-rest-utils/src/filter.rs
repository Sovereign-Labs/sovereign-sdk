//! Event filtering utilities for WebSocket streams.

use std::fmt::Debug;
use std::str::FromStr;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{json_obj, ErrorObject};

const MAX_FILTER_SIZE: usize = 4096;
const MAX_PATTERN_SIZE: usize = 1024;
const MAX_PATTERNS: usize = 128;

/// A filter for event streams that supports wildcard matching.
///
/// Supports the following patterns:
/// - Exact matches: `Bank/Transfer`
/// - Module wildcards: `Bank/*` (matches all events from the Bank module)
/// - Event wildcards: `*/Transfer` (matches Transfer events from any module)
/// - Multiple filters: `Bank/Transfer,Accounts/*`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    patterns: Vec<FilterPattern>,
}

impl Filter {
    /// Creates a new filter from a comma-separated string of patterns.
    pub fn new(filter_str: &str) -> Result<Self, FilterError> {
        if filter_str.len() > MAX_FILTER_SIZE {
            return Err(FilterError::FilterTooLarge);
        }
        if filter_str.is_empty() {
            return Ok(Self {
                patterns: Vec::new(),
            });
        }

        let patterns = filter_str
            .split(',')
            .map(|pattern| pattern.trim().parse())
            .collect::<Result<Vec<_>, _>>()?;
        if patterns.len() > MAX_PATTERNS {
            return Err(FilterError::TooManyPatterns);
        }

        Ok(Self { patterns })
    }

    /// Checks if the given event key matches any of the filter patterns.
    pub fn matches(&self, event_key: &str) -> bool {
        // If no patterns are specified, match everything
        if self.patterns.is_empty() {
            return true;
        }

        self.patterns
            .iter()
            .any(|pattern| pattern.matches(event_key))
    }
}

impl FromStr for Filter {
    type Err = FilterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum FilterPattern {
    /// Exact match: `Bank/Transfer`
    Exact { value: String },
    /// Module wildcard: `Bank/*`
    PrefixWithWildcard { prefix: String },
}

impl FilterPattern {
    fn matches(&self, event_key: &str) -> bool {
        match self {
            FilterPattern::Exact { value } => event_key.eq_ignore_ascii_case(value),
            FilterPattern::PrefixWithWildcard { prefix } => {
                event_key.len() >= prefix.len()
                    && event_key.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
            }
        }
    }
}

impl FromStr for FilterPattern {
    type Err = FilterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_PATTERN_SIZE {
            return Err(FilterError::FilterTooLarge);
        }
        if s.ends_with("*") {
            Ok(FilterPattern::PrefixWithWildcard {
                prefix: s.trim_end_matches("*").to_string(),
            })
        } else {
            Ok(FilterPattern::Exact {
                value: s.to_string(),
            })
        }
    }
}

/// Errors that can occur when parsing filter patterns.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    /// The provided filter is too large
    #[error("The provided filter is too large; individual filters must be less than {MAX_PATTERN_SIZE} characters and the total filter must be less than {MAX_FILTER_SIZE} characters")]
    FilterTooLarge,
    /// The provided filter contains too many patterns
    #[error("The provided filter contains too many patterns; individual filters must match less than {MAX_PATTERNS} patterns")]
    TooManyPatterns,
}

/// Query parameter extractor for event filters.
///
/// Example usage:
/// ```rust,ignore
/// use axum::extract::Query;
/// use sov_rest_utils::FilterQuery;
///
/// async fn websocket_handler(Query(filter): Query<FilterQuery>) {
///     // Use filter.0 to access the Filter
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterQuery {
    /// The filter parameter from the query string.
    #[serde(default)]
    pub filter: Option<Filter>,
}

impl FilterQuery {
    /// Gets the filter, returning a default (match-all) filter if none is specified.
    pub fn get_filter(&self) -> Filter {
        self.filter.clone().unwrap_or_else(|| Filter {
            patterns: Vec::new(),
        })
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for FilterQuery
where
    S: Send + Sync,
{
    type Rejection = ErrorObject;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Parse query parameters manually to handle the filter parameter
        let query_str = parts.uri.query().unwrap_or("");

        // Simple parsing for the filter parameter
        let filter = if let Some(filter_value) = extract_filter_param(query_str) {
            Some(Filter::new(&filter_value).map_err(|err| ErrorObject {
                status: StatusCode::BAD_REQUEST,
                message: "Invalid filter parameter".to_string(),
                details: json_obj!({
                    "filter": filter_value,
                    "error": err.to_string(),
                }),
            })?)
        } else {
            None
        };

        Ok(FilterQuery { filter })
    }
}

fn extract_filter_param(query_str: &str) -> Option<String> {
    // Parse the query string into a map
    let parsed: std::collections::HashMap<String, String> =
        serde_urlencoded::from_str(query_str).ok()?;

    parsed.get("key").cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let filter = Filter::new("Bank/Transfer").unwrap();
        assert!(filter.matches("bank/Transfer"));
        assert!(!filter.matches("Bank/CreateToken"));
        assert!(!filter.matches("Accounts/Transfer"));
    }

    #[test]
    fn test_prefix_with_wildcard() {
        let filter = Filter::new("Bank/*").unwrap();
        assert!(filter.matches("bank/transfer"));
        assert!(filter.matches("Bank/CreateToken"));
        assert!(!filter.matches("Accounts/Transfer"));
    }

    #[test]
    fn test_multiple_filters() {
        let filter = Filter::new("Bank/Transfer,Accounts/*").unwrap();
        assert!(filter.matches("Bank/Transfer"));
        assert!(filter.matches("Accounts/Register"));
        assert!(filter.matches("Accounts/Update"));
        assert!(!filter.matches("Bank/CreateToken"));
        assert!(!filter.matches("Other/Event"));
    }

    #[test]
    fn test_empty_filter() {
        let filter = Filter::new("").unwrap();
        assert!(filter.matches("Bank/Transfer"));
        assert!(filter.matches("Any/Event"));
    }

    #[test]
    fn test_filter_query_default() {
        let query = FilterQuery { filter: None };
        let filter = query.get_filter();
        assert!(filter.matches("Any/Event"));
    }
}
