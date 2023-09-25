use serde::{Deserialize, Serialize};
use sov_risc0_adapter::host::Risc0Host;
use sov_rollup_interface::zk::{ZkvmGuest, ZkvmHost};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TestStruct {
    ints: Vec<i32>,
    string: String,
}

#[test]
fn test_hints_roundtrip() {
    let hint = TestStruct {
        ints: vec![1, 2, 3, 4, 5],
        string: "hello".to_string(),
    };
    let mut host = Risc0Host::new(&[]);

    host.add_hint(&hint);

    let guest = host.simulate_with_hints();
    let received = guest.read_from_host();
    assert_eq!(hint, received);
}
