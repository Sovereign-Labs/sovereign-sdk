//! Test safe vec as its own binary to prevent the custom allocator from leaking to other tests.

// Use a custom allocator for testing to catch any instances where we erroneously assume that self.capacity < MAX_SIZE
// Taken from https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html.
//
// Add a redundant config gate with `cfg_test` to guard against copy-paste errors if this code gets moved later.
#[cfg(test)]
mod custom_allocator_that_over_allocates {
    use std::ptr::null_mut;
    use std::sync::atomic::Ordering::Relaxed;

    #[cfg(not(debug_assertions))]
    compile_error!(
        "Overrode global allocator with a test allocator inside of a release build! This is a bug!"
    );

    const ARENA_SIZE: usize = 30 * 1024 * 1024;
    const MAX_SUPPORTED_ALIGN: usize = 4096;
    #[repr(C, align(4096))] // 4096 == MAX_SUPPORTED_ALIGN
    struct SimpleAllocator {
        arena: UnsafeCell<[u8; ARENA_SIZE]>,
        remaining: AtomicUsize, // we allocate from the top, counting down
    }

    #[global_allocator]
    static ALLOCATOR: SimpleAllocator = SimpleAllocator {
        arena: UnsafeCell::new([0x55; ARENA_SIZE]),
        remaining: AtomicUsize::new(ARENA_SIZE),
    };

    unsafe impl Sync for SimpleAllocator {}

    unsafe impl GlobalAlloc for SimpleAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let mut size = layout.size();
            // purposefully over allocate by 10% for testing purposes
            size += size / 10;
            let align = layout.align();

            // `Layout` contract forbids making a `Layout` with align=0, or align not power of 2.
            // So we can safely use a mask to ensure alignment without worrying about UB.
            let align_mask_to_round_down = !(align - 1);

            if align > MAX_SUPPORTED_ALIGN {
                return null_mut();
            }

            let mut allocated = 0;
            let on_fetch_update = |mut remaining| {
                if size > remaining {
                    return None;
                }
                remaining -= size;
                remaining &= align_mask_to_round_down;
                allocated = remaining;
                Some(remaining)
            };
            if self
                .remaining
                .fetch_update(Relaxed, Relaxed, on_fetch_update)
                .is_err()
            {
                return null_mut();
            };
            self.arena.get().cast::<u8>().add(allocated)
        }
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    use std::alloc::{GlobalAlloc, Layout};
    use std::cell::UnsafeCell;
    use std::sync::atomic::AtomicUsize;
}

use sov_rollup_interface::common::SafeVec;

macro_rules! safe_vec {
        () => {
            SafeVec::new();
        };
        ($elem:expr; $n:expr) => {
            vec![$elem; $n].try_into().unwrap()
        };
        ($($x:expr),+ $(,)?) => {
            vec![$($x),+].try_into().unwrap()
        };
    }

#[test]
fn test_try_insert_doesnt_panic() {
    let mut v = SafeVec::<_, 1000>::new();
    assert_eq!(v.len(), 0);
    for i in 0..1000 {
        assert!(v.try_insert(i, 1).is_ok());
    }
    assert!(v.try_insert(0, 0).is_err());
    assert_eq!(v.len(), 1000);
}

#[test]
fn test_try_push_doesnt_panic() {
    let mut v = SafeVec::<_, 1000>::new();
    assert_eq!(v.len(), 0);
    for i in 0..1000 {
        assert!(v.try_push(i).is_ok());
    }
    assert!(v.try_push(0).is_err());
    assert_eq!(v.len(), 1000);
}

#[test]
fn test_try_from_vec() {
    let v = vec![1u8; 100];
    assert!(TryInto::<SafeVec<u8, 99>>::try_into(v.clone()).is_err());
    assert!(TryInto::<SafeVec<u8, 100>>::try_into(v.clone()).is_ok());
    assert!(TryInto::<SafeVec<u8, 101>>::try_into(v).is_ok());
}

#[test]
fn test_try_from_slice() {
    let v = vec![1u8; 100];
    assert!(TryInto::<SafeVec<u8, 99>>::try_into(v.as_slice()).is_err());
    assert!(TryInto::<SafeVec<u8, 100>>::try_into(v.as_slice()).is_ok());
    assert!(TryInto::<SafeVec<u8, 101>>::try_into(v.as_slice()).is_ok());
}

#[test]
fn test_try_append() {
    let mut v: SafeVec<_, 10> = SafeVec::new();
    assert!(v.try_append(&mut vec![0u8; 5]).is_ok());
    assert!(v.try_append(&mut vec![0u8; 5]).is_ok());
    assert!(v.try_append(&mut vec![0u8; 1]).is_err());
}

#[test]
fn test_try_extend_from_slice() {
    let mut v: SafeVec<_, 10> = SafeVec::new();
    assert!(v.try_extend_from_slice(&[0u8; 5]).is_ok());
    assert!(v.try_extend_from_slice(&[0u8; 5]).is_ok());
    assert!(v.try_extend_from_slice(&[0u8; 1]).is_err());
}

#[test]
fn test_try_extend_from_within() {
    let mut v: SafeVec<_, 10> = SafeVec::new();
    assert!(v.try_extend_from_slice(&[0u8; 5]).is_ok());
    assert!(v.try_extend_from_within(0..5).is_ok());
    assert!(v.try_extend_from_within(0..1).is_err());
}

#[test]
fn test_serde_json_roundtrips() {
    const BIG: usize = 1_000_001;
    let vec: SafeVec<_, BIG> = safe_vec![7u8; BIG];

    let serialized = &serde_json::to_vec(&vec).unwrap();
    let output: SafeVec<_, BIG> = serde_json::from_slice(serialized).unwrap();
    assert_eq!(vec, output);

    let way_too_small: Result<SafeVec<u8, 1>, _> = serde_json::from_slice(serialized);
    assert!(way_too_small.is_err());

    let too_small: Result<SafeVec<u8, 1_000_000>, _> = serde_json::from_slice(serialized);
    assert!(too_small.is_err());

    let too_big: Result<SafeVec<u8, 1_000_002>, _> = serde_json::from_slice(serialized);
    assert!(too_big.is_ok());
}

#[test]
fn test_bincode_roundtrips() {
    const BIG: usize = 1_000_001;
    let vec: SafeVec<_, BIG> = safe_vec![7u8; BIG];

    let serialized = &bincode::serialize(&vec).unwrap();
    let output: SafeVec<_, BIG> = bincode::deserialize(serialized).unwrap();
    assert_eq!(vec, output);

    let way_too_small: Result<SafeVec<u8, 1>, _> = bincode::deserialize(serialized);
    assert!(way_too_small.is_err());

    let too_small: Result<SafeVec<u8, 1_000_000>, _> = bincode::deserialize(serialized);
    assert!(too_small.is_err());

    let too_big: Result<SafeVec<u8, 1_000_002>, _> = bincode::deserialize(serialized);
    assert!(too_big.is_ok());
}

#[test]
fn test_borsh_roundtrips() {
    const BIG: usize = 1_000_001;
    let vec: SafeVec<_, BIG> = safe_vec![7u8; BIG];

    let serialized = &borsh::to_vec(&vec).unwrap();
    let output: SafeVec<_, BIG> = borsh::from_slice(serialized).unwrap();
    assert_eq!(vec, output);

    let way_too_small: Result<SafeVec<u8, 1>, _> = borsh::from_slice(serialized);
    assert!(way_too_small.is_err());

    let too_small: Result<SafeVec<u8, 1_000_000>, _> = borsh::from_slice(serialized);
    assert!(too_small.is_err());

    let too_big: Result<SafeVec<u8, 1_000_002>, _> = borsh::from_slice(serialized);
    assert!(too_big.is_ok());
}
