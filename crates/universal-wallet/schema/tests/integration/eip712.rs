use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::common::SafeString;
use sov_universal_wallet::schema::Schema;
use sov_universal_wallet::UniversalWallet;

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct Address([u8; 32]);
#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct TokenId([u8; 32]);

macro_rules! eip712_tests {
    // Default to comparing pretty-printed JSON
    ($schema_type:ty, $item:ident, $expected_display:literal) => {
        eip712_tests!($schema_type, $item, true, $expected_display);
    };
    ($schema_type:ty, $item:ident, $pretty_print:literal, $expected_display:literal) => {
        let schema = Schema::of_single_type::<$schema_type>().unwrap();
        let borsh_ser = borsh::to_vec(&$item).unwrap();
        let eip712_json = schema.eip712_json(0, &borsh_ser).unwrap();

        let eip712_json = if $pretty_print {
            // This allows passing $expected_display as a human-readable string, making the test
            // code easier to parse for humans
            let parsed: serde_json::Value = serde_json::from_str(&eip712_json).unwrap();
            serde_json::to_string_pretty(&parsed).unwrap()
        } else {
            eip712_json
        };
        // println!("{}", eip712_json); // Uncomment for debugging tests
        assert_eq!(eip712_json, $expected_display);

        // Assert hash can be calculated with no errors
        let _ = schema.eip712_signing_hash(0, &borsh_ser).unwrap();
    };
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
enum TestCallMessage {
    Transfer { amount: u128, to: Address },
    Mint { token: TokenId, amount: u128 },
}

#[test]
fn test_call_message() {
    let message = TestCallMessage::Transfer {
        amount: 56,
        to: Address([2u8; 32]),
    };

    eip712_tests!(
        TestCallMessage,
        message,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x7eb69c66ea4b5b2240aade5bbb135fccc00e4bce2fc20862222309ae97c542cc"
  },
  "message": {
    "Transfer": {
      "amount": "56",
      "to": "0x0202020202020202020202020202020202020202020202020202020202020202"
    }
  },
  "primaryType": "TestCallMessage",
  "types": {
    "TestCallMessage": [
      {
        "name": "Transfer",
        "type": "Transfer"
      }
    ],
    "Transfer": [
      {
        "name": "amount",
        "type": "uint128"
      },
      {
        "name": "to",
        "type": "string"
      }
    ]
  }
}"#
    );
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
enum Role {
    Attester,
    Prover { address: Address },
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct Msg {
    m: SafeString,
    role: Role,
}

#[test]
fn test_unit_enums() {
    let msg = Msg {
        m: "one".to_string().try_into().unwrap(),
        role: Role::Attester,
    };

    eip712_tests!(
        Msg,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x07b104a215fcba925e78c1ced02d15c3577b1fbce2b58daa5c5406beb6f99fcd"
  },
  "message": {
    "m": "one",
    "role": {
      "Attester": true
    }
  },
  "primaryType": "Msg",
  "types": {
    "Msg": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "Role"
      }
    ],
    "Role": [
      {
        "name": "Attester",
        "type": "bool"
      }
    ]
  }
}"#
    );
}

#[test]
fn test_enums_2() {
    let msg = Msg {
        m: "two".to_string().try_into().unwrap(),
        role: Role::Prover {
            address: Address([3u8; 32]),
        },
    };

    eip712_tests!(
        Msg,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x07b104a215fcba925e78c1ced02d15c3577b1fbce2b58daa5c5406beb6f99fcd"
  },
  "message": {
    "m": "two",
    "role": {
      "Prover": {
        "address": "0x0303030303030303030303030303030303030303030303030303030303030303"
      }
    }
  },
  "primaryType": "Msg",
  "types": {
    "Msg": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "Role"
      }
    ],
    "Prover": [
      {
        "name": "address",
        "type": "string"
      }
    ],
    "Role": [
      {
        "name": "Prover",
        "type": "Prover"
      }
    ]
  }
}"#
    );
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
enum RoleNested {
    Attester(u64),
    Prover(ProverType),
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
enum ProverType {
    Onchain { address: u64 },
    Offchain(u64),
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct MsgNested {
    m: SafeString,
    role: RoleNested,
}

#[test]
fn test_nested_enums() {
    let msg = MsgNested {
        m: "one".to_string().try_into().unwrap(),
        role: RoleNested::Attester(48),
    };

    eip712_tests!(
        MsgNested,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x07170be56d0d940836231cdae295585cd451c2cf74284bb07bd41317246f9be8"
  },
  "message": {
    "m": "one",
    "role": {
      "Attester": "48"
    }
  },
  "primaryType": "MsgNested",
  "types": {
    "MsgNested": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "RoleNested"
      }
    ],
    "RoleNested": [
      {
        "name": "Attester",
        "type": "uint64"
      }
    ]
  }
}"#
    );
}

#[test]
fn test_nested_enums_2() {
    let msg = MsgNested {
        m: "two".to_string().try_into().unwrap(),
        role: RoleNested::Prover(ProverType::Onchain { address: 65 }),
    };

    eip712_tests!(
        MsgNested,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x07170be56d0d940836231cdae295585cd451c2cf74284bb07bd41317246f9be8"
  },
  "message": {
    "m": "two",
    "role": {
      "Prover": {
        "Onchain": {
          "address": "65"
        }
      }
    }
  },
  "primaryType": "MsgNested",
  "types": {
    "MsgNested": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "RoleNested"
      }
    ],
    "Onchain": [
      {
        "name": "address",
        "type": "uint64"
      }
    ],
    "Prover": [
      {
        "name": "Onchain",
        "type": "Onchain"
      }
    ],
    "RoleNested": [
      {
        "name": "Prover",
        "type": "Prover"
      }
    ]
  }
}"#
    );
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
enum RoleMultiElement {
    Attester(u64, u128),
    Prover(ProverType, u128),
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct MsgMultiElement {
    m: SafeString,
    role: RoleMultiElement,
}

#[test]
fn test_multielement_enums_1() {
    let msg = MsgMultiElement {
        m: "one".to_string().try_into().unwrap(),
        role: RoleMultiElement::Attester(765, 9876),
    };

    eip712_tests!(
        MsgMultiElement,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0xa0a9f6d015a69364793d27147f6a3ccec18bd99eff57689706804dbb944c7192"
  },
  "message": {
    "m": "one",
    "role": {
      "Attester": {
        "0": "765",
        "1": "9876"
      }
    }
  },
  "primaryType": "MsgMultiElement",
  "types": {
    "Attester": [
      {
        "name": "0",
        "type": "uint64"
      },
      {
        "name": "1",
        "type": "uint128"
      }
    ],
    "MsgMultiElement": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "RoleMultiElement"
      }
    ],
    "RoleMultiElement": [
      {
        "name": "Attester",
        "type": "Attester"
      }
    ]
  }
}"#
    );
}

#[test]
fn test_multielement_enums_2() {
    let msg = MsgMultiElement {
        m: "two".to_string().try_into().unwrap(),
        role: RoleMultiElement::Prover(ProverType::Onchain { address: 123456 }, 9876),
    };

    eip712_tests!(
        MsgMultiElement,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0xa0a9f6d015a69364793d27147f6a3ccec18bd99eff57689706804dbb944c7192"
  },
  "message": {
    "m": "two",
    "role": {
      "Prover": {
        "0": {
          "Onchain": {
            "address": "123456"
          }
        },
        "1": "9876"
      }
    }
  },
  "primaryType": "MsgMultiElement",
  "types": {
    "MsgMultiElement": [
      {
        "name": "m",
        "type": "string"
      },
      {
        "name": "role",
        "type": "RoleMultiElement"
      }
    ],
    "Onchain": [
      {
        "name": "address",
        "type": "uint64"
      }
    ],
    "Prover": [
      {
        "name": "0",
        "type": "ProverType"
      },
      {
        "name": "1",
        "type": "uint128"
      }
    ],
    "ProverType": [
      {
        "name": "Onchain",
        "type": "Onchain"
      }
    ],
    "RoleMultiElement": [
      {
        "name": "Prover",
        "type": "Prover"
      }
    ]
  }
}"#
    );
}

#[derive(BorshSerialize, BorshDeserialize, UniversalWallet)]
struct MsgVariousTypes {
    u8: u8,
    u16: u16,
    u32: u32,
    u64: u64,
    u128: u128,
    i8: i8,
    i16: i16,
    i32: i32,
    i64: i64,
    i128: i128,
    bool: bool,
    f32: f32,
    f64: f64,
    string: SafeString,
    byte_vec: Vec<u8>,
    byte_array: [u8; 4],
    array: [bool; 3],
    vec: Vec<bool>,
    map: BTreeMap<u8, bool>,
}

#[test]
fn test_various_types() {
    let msg = MsgVariousTypes {
        u8: 92,
        u16: 392,
        u32: 15_472_432,
        u64: 340_542_814_143,
        u128: 180_446_744_073_709_551_615,
        i8: -92,
        i16: -392,
        i32: -15_472_432,
        i64: -340_542_814_143,
        i128: -180_446_744_073_709_551_615,
        bool: true,
        f32: 45.59,
        f64: 9716235.31632546,
        string: "Hello".to_string().try_into().unwrap(),
        byte_vec: vec![6, 7, 8],
        byte_array: [3, 5, 7, 9],
        vec: vec![true, false, true],
        array: [false, false, true],
        map: BTreeMap::from([(1, true), (2, false), (3, true)]),
    };

    eip712_tests!(
        MsgVariousTypes,
        msg,
        r#"{
  "domain": {
    "chainId": "0x0",
    "name": "",
    "salt": "0x387d7c4b3c3e545f6e6d1cad1b606ee46250d3acd1dfa80226d0e05551f20527"
  },
  "message": {
    "array": {
      "0": false,
      "1": false,
      "2": true
    },
    "bool": true,
    "byte_array": "0x03050709",
    "byte_vec": "0x060708",
    "f32": "45.59",
    "f64": "9716235.31632546",
    "i128": "-180446744073709551615",
    "i16": "-392",
    "i32": "-15472432",
    "i64": "-340542814143",
    "i8": "-92",
    "map": {
      "1": true,
      "2": false,
      "3": true
    },
    "string": "Hello",
    "u128": "180446744073709551615",
    "u16": "392",
    "u32": "15472432",
    "u64": "340542814143",
    "u8": "92",
    "vec": {
      "0": true,
      "1": false,
      "2": true
    }
  },
  "primaryType": "MsgVariousTypes",
  "types": {
    "MsgVariousTypes": [
      {
        "name": "u8",
        "type": "uint8"
      },
      {
        "name": "u16",
        "type": "uint16"
      },
      {
        "name": "u32",
        "type": "uint32"
      },
      {
        "name": "u64",
        "type": "uint64"
      },
      {
        "name": "u128",
        "type": "uint128"
      },
      {
        "name": "i8",
        "type": "int8"
      },
      {
        "name": "i16",
        "type": "int16"
      },
      {
        "name": "i32",
        "type": "int32"
      },
      {
        "name": "i64",
        "type": "int64"
      },
      {
        "name": "i128",
        "type": "int128"
      },
      {
        "name": "bool",
        "type": "bool"
      },
      {
        "name": "f32",
        "type": "string"
      },
      {
        "name": "f64",
        "type": "string"
      },
      {
        "name": "string",
        "type": "string"
      },
      {
        "name": "byte_vec",
        "type": "string"
      },
      {
        "name": "byte_array",
        "type": "string"
      },
      {
        "name": "array",
        "type": "array"
      },
      {
        "name": "vec",
        "type": "vec"
      },
      {
        "name": "map",
        "type": "map"
      }
    ],
    "array": [
      {
        "name": "0",
        "type": "bool"
      },
      {
        "name": "1",
        "type": "bool"
      },
      {
        "name": "2",
        "type": "bool"
      }
    ],
    "map": [
      {
        "name": "1",
        "type": "bool"
      },
      {
        "name": "2",
        "type": "bool"
      },
      {
        "name": "3",
        "type": "bool"
      }
    ],
    "vec": [
      {
        "name": "0",
        "type": "bool"
      },
      {
        "name": "1",
        "type": "bool"
      },
      {
        "name": "2",
        "type": "bool"
      }
    ]
  }
}"#
    );
}
