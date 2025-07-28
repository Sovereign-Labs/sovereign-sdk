#![allow(missing_docs)]
//! To check that [`NativeStorageManager`] creates correct [`DeltaReader`]
//! tests are writing data related to each block,
//! so it can be validated by looking at what data reader can provide.

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_mock_da::{MockBlockHeader, MockHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::stf::StoredEvent;

use crate::accessory_db::AccessoryDb;
use crate::schema::tables::EventByNumber;
use crate::schema::types::EventNumber;
// Encoding/Decoding data.

pub(crate) type H = sha2::Sha256;

fn decode_ledger_item(item: (EventNumber, StoredEvent)) -> (u64, MockHash) {
    let (event_number, stored_event) = item;
    let height = event_number.0;
    assert_eq!(stored_event.key().inner(), stored_event.value().inner());
    let da_hash = MockHash::try_from(stored_event.key().inner().to_vec()).unwrap();
    (height, da_hash)
}

/// Using [`MockBlockHeader::height`] as a key and [`MockHash`] of header as value.
///
/// What it writes:
///  - [`EventByNumber`] => `block_height` => Event {key == block_hash, value == block_hash}
///
/// So it can be validated by traversing data from DB.
pub fn materialize_ledger_changes(da_header: &MockBlockHeader) -> SchemaBatch {
    let mut change_set = SchemaBatch::default();
    let key = &EventNumber(da_header.height());
    let value = StoredEvent::new(&da_header.hash().0, &da_header.hash().0, da_header.hash().0);

    change_set.put::<EventByNumber>(key, &value).unwrap();

    change_set
}

#[allow(missing_docs)]
pub fn verify_accessory_db(accessory_db: &AccessoryDb, expected_values: &[(u64, MockHash)]) {
    for (expected_height, expected_hash) in expected_values {
        let key = expected_height.to_be_bytes().to_vec();
        let actual_value = accessory_db
            .get_value_option(&key, SlotNumber::GENESIS)
            .unwrap()
            .expect("Missing value in AccessoryDb");
        assert_eq!(expected_hash.0.to_vec(), actual_value);
    }
}

#[allow(missing_docs)]
pub fn verify_ledger_storage(reader: &DeltaReader, expected_values: &[(u64, MockHash)]) {
    let range = EventNumber(0)..EventNumber(u64::MAX);

    let actual_values: Vec<(u64, MockHash)> = reader
        .collect_in_range::<EventByNumber, _>(range)
        .unwrap()
        .into_iter()
        .map(decode_ledger_item)
        .collect();
    assert_eq!(expected_values, &actual_values);
}

#[allow(missing_docs)]
pub fn get_expected_chain_values(processed_chain: &[MockBlockHeader]) -> Vec<(u64, MockHash)> {
    processed_chain
        .iter()
        .map(|b| (b.height(), b.hash()))
        .collect()
}
