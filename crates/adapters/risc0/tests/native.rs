use std::fmt::Debug;

use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::zk::{ZkvmGuest, ZkvmHost};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TestStruct {
    ints: Vec<i32>,
    string: String,
}

fn check_hints_round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(hints: Vec<T>) {
    let mut host = Risc0Host::new(&[]);

    for hint in &hints {
        host.add_hint(hint);
    }

    let guest = host.simulate_with_hints();

    for hint in &hints {
        println!("TRY READING: {hint:?}");
        let received = guest.read_from_host();
        assert_eq!(hint, &received);
    }
}

#[test]
fn test_hints_round_trip() {
    check_hints_round_trip(vec![
        TestStruct {
            ints: vec![1, 2, 3, 4, 5],
            string: "hello".to_string(),
        },
        TestStruct {
            ints: vec![10, -20, 30, 49, 50],
            string: "hello B".to_string(),
        },
    ]);
}

proptest! {

    #[test]
    fn test_hex_hash_roundtrip(item in any::<sov_rollup_interface::common::HexHash>()) {
        check_hints_round_trip(vec![item]);
    }

    #[test]
    fn test_hex_string_roundtrip(item in any::<sov_rollup_interface::common::HexString>()) {
        check_hints_round_trip(vec![item]);
    }
}
