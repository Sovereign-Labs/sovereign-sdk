use std::ops::RangeInclusive;

/// An empty range used to represent no new items.
#[allow(clippy::reversed_empty_ranges)]
const EMPTY_RANGE: RangeInclusive<u64> = 1..=0;

/// Tracks the high-water mark of a monotonically increasing sequence.
///
/// Returns new items above the watermark on each advance.
/// https://en.wikipedia.org/wiki/Watermark_(data_synchronization)
pub struct Watermark {
    mark: u64,
}

impl Watermark {
    pub fn new(initial: u64) -> Self {
        Self { mark: initial }
    }

    /// Returns the range of new items from the last mark to the new value.
    ///
    /// If the new value hasn't advanced past the current mark, returns an empty range.
    /// Otherwise, updates the mark and returns the range of new items.
    pub fn advance(&mut self, new_value: u64) -> RangeInclusive<u64> {
        if new_value <= self.mark {
            return EMPTY_RANGE;
        }
        let range = self.mark + 1..=new_value;
        self.mark = new_value;
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
        assert_eq!(range, 1..=2);
        assert_eq!(watermark.mark, 2);
    }

    #[test]
    fn no_change_returns_empty() {
        let mut watermark = Watermark::new(0);

        let range = watermark.advance(0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.mark, 0);
    }

    #[test]
    fn backward_movement_returns_empty() {
        let mut watermark = Watermark::new(1);

        let range = watermark.advance(0);
        assert_eq!(range, EMPTY_RANGE);
        assert_eq!(watermark.mark, 1); // Mark doesn't move backward
    }

    #[test]
    fn multiple_advances() {
        let mut watermark = Watermark::new(0);

        assert_eq!(watermark.advance(3), 1..=3);
        assert_eq!(watermark.advance(3), EMPTY_RANGE);
        assert_eq!(watermark.advance(5), 4..=5);
        assert_eq!(watermark.advance(5), EMPTY_RANGE);
        assert_eq!(watermark.advance(7), 6..=7);
    }

    #[test]
    fn single_item_increment() {
        let mut watermark = Watermark::new(0);

        assert_eq!(watermark.advance(1), 1..=1);
        assert_eq!(watermark.advance(2), 2..=2);
    }
}
