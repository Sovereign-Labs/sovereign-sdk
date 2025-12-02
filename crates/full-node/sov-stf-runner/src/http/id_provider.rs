use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use jsonrpsee::{server::IdProvider, types::SubscriptionId};

#[derive(Default, Debug)]
pub(crate) struct HexIdProvider(AtomicU64);

impl IdProvider for HexIdProvider {
    fn next_id(&self) -> SubscriptionId<'static> {
        let id = self.0.fetch_add(1, Relaxed);
        format!("0x{id:016x}").into()
    }
}

#[test]
fn test_hex_id_provider() {
    let provider = HexIdProvider::default();
    assert_eq!(
        provider.next_id(),
        SubscriptionId::Str("0x0000000000000000".into())
    );
    assert_eq!(
        provider.next_id(),
        SubscriptionId::Str("0x0000000000000001".into())
    );
}
