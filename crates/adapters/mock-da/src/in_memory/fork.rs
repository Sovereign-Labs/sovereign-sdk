/// Definition of a fork that will be executed by [`crate::MockDaService`] at a
/// specified height.
#[derive(Clone, Debug)]
pub struct PlannedFork {
    pub(crate) trigger_at_height: u64,
    pub(crate) fork_height: u64,
    pub(crate) blobs: Vec<Vec<u8>>,
}

impl PlannedFork {
    /// Creates new [`PlannedFork`]. Panics if some parameters are invalid.
    ///
    /// # Arguments
    ///
    /// * `trigger_at_height` - Height at which fork is "noticed".
    /// * `fork_height` - Height at which the chain forked. The height of the first block in `blobs` will be `fork_height + 1`
    /// * `blobs` - Blobs that will be added after fork. Single blob per each block.
    ///   Blobs length needs be larger than difference between trigger_at_height and fork_height, otherwise there would be on block available at `trigger_at_height`
    ///
    /// ```text
    /// ----- visual example:
    /// height    1    2    3    4    5    6    7    8
    /// blocks    a -> b -> c -> d -> e -> f -> g
    /// blocks                   \ -> h -> k -> l -> m
    /// ------
    /// blobs.len(): 3
    /// trigger_at_height: 7
    /// fork_height: 4
    /// ```
    pub fn new(trigger_at_height: u64, fork_height: u64, blobs: Vec<Vec<u8>>) -> Self {
        if fork_height > trigger_at_height {
            panic!("Fork height must be less than trigger height");
        }
        let fork_len = (trigger_at_height - fork_height) as usize;
        if blobs.len() < fork_len {
            panic!(
                "Not enough blobs for fork to be produced at given height, fork_len={} blobs={}",
                fork_len,
                blobs.len()
            );
        }
        Self {
            trigger_at_height,
            fork_height,
            blobs,
        }
    }
}
