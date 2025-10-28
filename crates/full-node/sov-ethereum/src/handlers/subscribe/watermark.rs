use std::ops::{Range, RangeTo};

/// An empty range used to represent no new items.
const EMPTY_RANGE: Range<u64> = 0..0;

/// Tracks the high-water mark and emits only new items on each advance.
///
/// Returns the range of items that are new since the last processed position.
/// https://en.wikipedia.org/wiki/Watermark_(data_synchronization)
pub struct Watermark {
    processed: RangeTo<u64>,
}

impl Watermark {
    pub fn new(processed: RangeTo<u64>) -> Self {
        Self { processed }
    }

    /// Returns the range of new items that haven't been processed yet.
    ///
    /// If the visible range hasn't advanced past the processed watermark, returns an empty range.
    /// Otherwise, updates the watermark and returns the range of new items.
    pub fn advance(&mut self, visible: RangeTo<u64>) -> Range<u64> {
        if visible.end <= self.processed.end {
            return EMPTY_RANGE;
        }
        let range = self.processed.end + 1..visible.end;
        self.processed = visible;
        range
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_progression() {
        let mut watermark = Watermark::new(..0);

        let range = watermark.advance(..2);
        assert_eq!(range, 1..2);
        assert_eq!(watermark.processed, ..2);
    }

    #[test]
    fn no_change_returns_empty() {
        let mut watermark = Watermark::new(..0);

        let range = watermark.advance(..0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.processed, ..0);
    }

    #[test]
    fn backward_movement_returns_empty() {
        let mut watermark = Watermark::new(..1);

        let range = watermark.advance(..0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.processed, ..1); // Last processed doesn't move backward
    }

    #[test]
    fn multiple_advances() {
        let mut watermark = Watermark::new(..0);

        assert_eq!(watermark.advance(..3), 1..3);
        assert_eq!(watermark.advance(..3), EMPTY_RANGE);
        assert_eq!(watermark.advance(..5), 4..5);
        assert_eq!(watermark.advance(..5), EMPTY_RANGE);
        assert_eq!(watermark.advance(..7), 6..7);
    }

    #[test]
    fn single_item_increment() {
        let mut watermark = Watermark::new(..0);

        assert_eq!(watermark.advance(..1), 1..1);
        assert_eq!(watermark.advance(..2), 2..2);
    }
}
