use std::ops::Range;

/// An empty range used to represent no new items.
const EMPTY_RANGE: Range<u64> = 0..0;

/// Tracks the high-water mark of a monotonically increasing sequence.
///
/// Returns new items above the watermark on each advance.
/// https://en.wikipedia.org/wiki/Watermark_(data_synchronization)
pub struct Watermark {
    last_processed: u64,
}

impl Watermark {
    pub fn new(last_processed: u64) -> Self {
        Self { last_processed }
    }

    /// Returns the range of new items from the last processed element to the new value.
    ///
    /// If the new value hasn't advanced past the last processed element, returns an empty range.
    /// Otherwise, updates the last processed element and returns the range of new items.
    pub fn advance(&mut self, latest: u64) -> Range<u64> {
        if latest <= self.last_processed {
            return EMPTY_RANGE;
        }
        let range = self.last_processed + 1..latest + 1;
        self.last_processed = latest;
        range
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_progression() {
        let mut watermark = Watermark::new(0);

        let range = watermark.advance(2);
        assert_eq!(range, 1..3);
        assert_eq!(watermark.last_processed, 2);
    }

    #[test]
    fn no_change_returns_empty() {
        let mut watermark = Watermark::new(0);

        let range = watermark.advance(0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.last_processed, 0);
    }

    #[test]
    fn backward_movement_returns_empty() {
        let mut watermark = Watermark::new(1);

        let range = watermark.advance(0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.last_processed, 1); // Last processed doesn't move backward
    }

    #[test]
    fn multiple_advances() {
        let mut watermark = Watermark::new(0);

        assert_eq!(watermark.advance(3), 1..4);
        assert_eq!(watermark.advance(3), EMPTY_RANGE);
        assert_eq!(watermark.advance(5), 4..6);
        assert_eq!(watermark.advance(5), EMPTY_RANGE);
        assert_eq!(watermark.advance(7), 6..8);
    }

    #[test]
    fn single_item_increment() {
        let mut watermark = Watermark::new(0);

        assert_eq!(watermark.advance(1), 1..2);
        assert_eq!(watermark.advance(2), 2..3);
    }
}
