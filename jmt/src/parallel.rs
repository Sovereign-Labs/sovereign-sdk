use crate::{
    hash::TreeHash, node_type::Node, types::nibble::Nibble, JmtError, Key, NibbleRangeIterator,
    TreeReader, TreeUpdateBatch, ValueHash,
};

#[cfg(any(test, feature = "rayon"))]
pub fn parallel_process_range_if_enabled<
    'a,
    R: TreeReader<K, H, N>,
    K: Key,
    H: TreeHash<N>,
    F,
    const N: usize,
>(
    depth: usize,
    range_iter: NibbleRangeIterator<Option<&(ValueHash<N>, K)>, N>,
    batch: &'a mut TreeUpdateBatch<K, H, N>,
    mapper: F,
) -> Result<Vec<(Nibble, Option<Node<K, H, N>>)>, JmtError<R::Error>>
where
    R: 'a,
    H: 'a,
    K: 'a,
    R::Error: 'a,

    for<'b> F: Fn(
            usize,
            usize,
            &'b mut TreeUpdateBatch<K, H, N>,
        ) -> Result<(Nibble, Option<Node<K, H, N>>), JmtError<R::Error>>
        + Send
        + Sync,
{
    use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

    const MAX_PARALLELIZABLE_DEPTH: usize = 2;
    if depth <= MAX_PARALLELIZABLE_DEPTH {
        Ok(range_iter
            .collect::<Vec<_>>()
            .par_iter()
            .map(|(left, right)| {
                let mut sub_batch = TreeUpdateBatch::<K, H, N>::new();
                Ok((mapper(*left, *right, &mut sub_batch)?, sub_batch))
            })
            .collect::<Result<Vec<_>, JmtError<R::Error>>>()?
            .into_iter()
            .map(|(ret, sub_batch)| {
                batch.combine(sub_batch);
                ret
            })
            .collect())
    } else {
        range_iter
            .map(|(left, right)| mapper(left, right, batch))
            .collect::<Result<_, JmtError<R::Error>>>()
    }
}

#[cfg(not(any(test, feature = "rayon")))]
pub fn parallel_process_range_if_enabled<
    'a,
    R: TreeReader<K, H, N>,
    K: Key,
    H: TreeHash<N>,
    F,
    const N: usize,
>(
    _depth: usize,
    range_iter: NibbleRangeIterator<Option<&(ValueHash<N>, K)>, N>,
    batch: &'a mut TreeUpdateBatch<K, H, N>,
    mapper: F,
) -> Result<Vec<(Nibble, Option<Node<K, H, N>>)>, JmtError<R::Error>>
where
    R: 'a,
    H: 'a,
    K: 'a,
    R::Error: 'a,
    for<'b> F: Fn(
            usize,
            usize,
            &'b mut TreeUpdateBatch<K, H, N>,
        ) -> Result<(Nibble, Option<Node<K, H, N>>), JmtError<R::Error>>
        + Send
        + Sync,
{
    range_iter
        .map(|(left, right)| mapper(left, right, batch))
        .collect::<Result<_, JmtError<R::Error>>>()
}

#[cfg(any(test, feature = "rayon"))]
const NUM_IO_THREADS: usize = 32;

#[cfg(any(test, feature = "rayon"))]
pub static IO_POOL: once_cell::sync::Lazy<rayon::ThreadPool> = once_cell::sync::Lazy::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(NUM_IO_THREADS)
        .thread_name(|index| format!("jmt-io-{}", index))
        .build()
        .unwrap()
});

#[cfg(not(any(test, feature = "rayon")))]
pub fn run_on_io_pool_if_enabled<OP, R>(op: OP) -> R
where
    OP: FnOnce() -> R + Send,
    R: Send,
{
    op()
}

#[cfg(any(test, feature = "rayon"))]
pub fn run_on_io_pool_if_enabled<OP, R>(op: OP) -> R
where
    OP: FnOnce() -> R + Send,
    R: Send,
{
    IO_POOL.install(op)
}
