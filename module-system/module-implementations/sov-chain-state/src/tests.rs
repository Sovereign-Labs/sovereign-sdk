use sov_modules_api::da::{NanoSeconds, Time};

use crate::ChainStateConfig;

#[test]
fn test_config_serialization() {
    let time = Time::new(2, NanoSeconds::new(3).unwrap());
    let config = ChainStateConfig {
        initial_slot_height: 1,
        current_time: time,
    };

    let data = r#"
    {
        "initial_slot_height":1,
        "current_time":{
            "secs":2,
            "nanos":3
        }
    }"#;

    let parsed_config: ChainStateConfig = serde_json::from_str(data).unwrap();
    assert_eq!(config, parsed_config)
}
