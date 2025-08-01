use avail_rust_core::HasTxDispatchIndex;

#[derive(codec::Decode, codec::Encode, PartialEq, Eq, Debug)]
pub struct CustomTransaction {
    #[codec(compact)]
    pub set: u64,
}
impl HasTxDispatchIndex for CustomTransaction {
    const DISPATCH_INDEX: (u8, u8) = (3u8, 0u8);
}

pub const KATE_START_TIME: i64 = 1686066440;
pub const KATE_SECONDS_PER_BLOCK: i64 = 20;
