use core::str::FromStr;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::Range;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use sov_universal_wallet::schema::safe_string::SafeString;
use sov_universal_wallet::schema::{
    ChainData, IndexLinking, Item, Link, Primitive, RollupRoots, Schema, UniversalWallet,
};
use sov_universal_wallet::UniversalWallet;

// Hack - because the macro is configured to be re-exported from sov_rollup_interface;
// but _we_ are a dependency of sov_rollup_interface so we can't import it without causing a cycle
// This should not be an issue anywhere else except inside this crate's tests right here
mod sov_rollup_interface {
    pub use sov_universal_wallet;
}

#[derive(Debug, Serialize)]
struct TestVector {
    /// The JSON object to be serialized to borsh
    input: String,
    /// The resulting borsh encoding as a hex string
    output: String,
    /// The universal wallet schema as a JSON string
    schema: String,
}

// TODO: there's probably a better way than nested macros
macro_rules! encode_decode_tests_simple {
    ($schema:ident, $item:ident, $expected_display:literal) => {
        let borsh_ser = borsh::to_vec(&$item).unwrap();
        let json = serde_json::to_string(&$item).unwrap();

        if cfg!(feature = "test-vectors") {
            use std::time::{SystemTime, UNIX_EPOCH};

            let timestamp_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::var("SOV_UNIVERSAL_WALLET_TEST_VECTORS_DIR")
                .unwrap_or_else(|_| "test-vectors".to_string());
            std::fs::create_dir_all(&dir).unwrap();

            let th = std::thread::current();
            let test_path = th.name().unwrap();
            let test_name = test_path
                .split("::")
                .last()
                .expect("this shouldnt be possible");
            let vector = TestVector {
                input: json.clone(),
                output: hex::encode(&borsh_ser),
                schema: serde_json::to_string(&$schema).unwrap(),
            };

            let file = format!("{dir}/{test_name}-{timestamp_nanos}.json");
            std::fs::write(file, serde_json::to_string_pretty(&vector).unwrap()).unwrap();
        }

        // println!("{}", json);
        assert_eq!($schema.display(0, &borsh_ser).unwrap(), $expected_display);
        assert_eq!($schema.json_to_borsh(0, &json).unwrap(), borsh_ser);
    };
}

macro_rules! encode_decode_tests {
    ($schema_type:ty, $item:ident, $expected_display:literal) => {
        let schema = Schema::of_single_type::<$schema_type>().unwrap();
        // println!("{:?}", &schema);
        encode_decode_tests_simple!(schema, $item, $expected_display);
        let chain_hash = schema.cached_chain_hash().unwrap();
        let schema_json = serde_json::to_string_pretty(&schema).unwrap();
        // println!("{schema_json}");
        let mut recovered_schema = Schema::from_json(&schema_json).unwrap();
        let recovered_chain_hash = recovered_schema.chain_hash().unwrap();
        assert_eq!(chain_hash, recovered_chain_hash);
        encode_decode_tests_simple!(recovered_schema, $item, $expected_display);
    };
}

pub trait Spec {
    type Address: UniversalWallet;
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithAssociatedType<S: Spec> {
    AssociatedVariant { address: S::Address },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithWhereClauseAssociatedType<S>
where
    S: Spec,
{
    TheVariant { address: S::Address },
}

#[test]
fn test_associated_types() {
    struct MySpec();
    impl Spec for MySpec {
        type Address = u64;
    }

    let my_enum = EnumWithAssociatedType::<MySpec>::AssociatedVariant { address: 123 };

    encode_decode_tests!(
        EnumWithAssociatedType<MySpec>,
        my_enum,
        "AssociatedVariant { address: 123 }"
    );

    let my_enum = EnumWithWhereClauseAssociatedType::<MySpec>::TheVariant { address: 123 };

    encode_decode_tests!(
        EnumWithWhereClauseAssociatedType<MySpec>,
        my_enum,
        "TheVariant { address: 123 }"
    );
}

#[test]
fn test_inner_item_derive() {
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
    #[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
    struct A {
        my_field: u8,
    }

    let my_a = A { my_field: 32 };
    encode_decode_tests!(A, my_a, "{ my_field: 32 }");
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[borsh(use_discriminant = true)]
pub enum SimpleEnumWithDiscriminants {
    First = 1,
    Second = 6,
    Third,
    Fourth = 100,
    Fifth = 3,
    Sixth,
    Seventh,
    Eighth = 0,
}

#[test]
fn test_simple_enum_with_discriminants() {
    let my_enum = SimpleEnumWithDiscriminants::First;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "First");
    let my_enum = SimpleEnumWithDiscriminants::Second;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Second");
    let my_enum = SimpleEnumWithDiscriminants::Third;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Third");
    let my_enum = SimpleEnumWithDiscriminants::Fourth;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Fourth");
    let my_enum = SimpleEnumWithDiscriminants::Fifth;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Fifth");
    let my_enum = SimpleEnumWithDiscriminants::Sixth;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Sixth");
    let my_enum = SimpleEnumWithDiscriminants::Seventh;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Seventh");
    let my_enum = SimpleEnumWithDiscriminants::Eighth;
    encode_decode_tests!(SimpleEnumWithDiscriminants, my_enum, "Eighth");
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[borsh(use_discriminant = false)]
pub enum EnumWithDiscriminantsDisabledInBorsh {
    First = 1,
    Second = 6,
    Third,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithDiscriminantsDisabledInBorshSurrogateWithoutDiscriminants {
    First,
    Second,
    Third,
}

#[test]
fn test_enum_with_discriminants_disabled_in_borsh() {
    let my_enum = EnumWithDiscriminantsDisabledInBorsh::First;
    encode_decode_tests!(EnumWithDiscriminantsDisabledInBorsh, my_enum, "First");
    let my_enum = EnumWithDiscriminantsDisabledInBorsh::Second;
    encode_decode_tests!(EnumWithDiscriminantsDisabledInBorsh, my_enum, "Second");
    let my_enum = EnumWithDiscriminantsDisabledInBorsh::Third;
    encode_decode_tests!(EnumWithDiscriminantsDisabledInBorsh, my_enum, "Third");

    // Custom test to make sure we serialize enums correctly without a discriminant
    let schema_surrogate = Schema::of_single_type::<
        EnumWithDiscriminantsDisabledInBorshSurrogateWithoutDiscriminants,
    >()
    .unwrap();
    let borsh_from_discriminants = borsh::to_vec(&my_enum).unwrap();
    let json_from_discriminants = serde_json::to_string(&my_enum).unwrap();
    assert_eq!(
        schema_surrogate
            .json_to_borsh(0, &json_from_discriminants)
            .unwrap(),
        borsh_from_discriminants
    );
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
/// A type which doesn't derive `UniversalWallet` and doesn't have a schema gen implementation
pub struct NoSchemaU64Wrapper(pub u64);

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[serde(rename_all = "snake_case")]
pub enum TestCall {
    Register(Registration),
    RegisterMany(Vec<RegistrationLike>),
    RegisterSimple(Vec<u8>),
    Withdraw(u64),
    #[serde(skip)]
    #[allow(unused)]
    // We need this variant to test the schema generation of recursive types even though we don't construct it
    Complex(#[cfg_attr(test, sov_wallet(bound = ""))] Box<Complex>),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[serde(rename_all = "snake_case")]
pub enum SimpleEnum {
    One(u8),
    Two(u32),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[serde(rename_all = "snake_case")]
#[sov_wallet(hide_tag)]
pub enum HideTagEnum {
    A(u64),
    B(SimpleEnum),
}

#[derive(Debug, Hash, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct MinimalStruct {
    tokens: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[cfg_attr(
    test,
    sov_wallet(
        show_as = "This is a simple struct, with {} tokens, and the following message: {}. End of template!"
    )
)]
pub struct SimpleStructWithShowAs {
    tokens: u64,
    msg: SafeString,
}

#[test]
fn test_hide_tag_enum() {
    let hide_tag = HideTagEnum::A(0);
    encode_decode_tests!(HideTagEnum, hide_tag, "0");

    let nested_hide_tag = HideTagEnum::B(SimpleEnum::One(4));
    encode_decode_tests!(HideTagEnum, nested_hide_tag, "SimpleEnum.One(4)");
}

#[test]
fn test_simple_struct_schema_with_showas() {
    let my_registration = SimpleStructWithShowAs {
        tokens: 1000,
        msg: "abc".to_string().try_into().unwrap(),
    };

    encode_decode_tests!(SimpleStructWithShowAs, my_registration, "This is a simple struct, with 1000 tokens, and the following message: \"abc\". End of template!");
}

#[derive(
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct SimpleStructWithTemplate {
    #[sov_wallet(template("transfer" = input("amount"), "transfer_2" = value(default)))]
    tokens: u64,
    #[sov_wallet(template("transfer" = value("ababab"), "transfer_2" = input))]
    msg: SafeString,
}

// trivial implementation for tests: comma-separated fields e.g. "4,afdsa"
impl FromStr for SimpleStructWithTemplate {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (tokens, msg) = s.split_once(',').unwrap();
        let tokens = tokens.parse().unwrap();
        let msg = msg.try_into().unwrap();
        Ok(Self { tokens, msg })
    }
}

#[derive(
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct SimpleStructWithTemplateAndDisplays {
    #[sov_wallet(template("transfer" = input("tokens"), "receive" = input("tokens")))]
    tokens: u64,
    #[sov_wallet(template("transfer" = input("to_hex"), "receive" = value(bytes("0x0808080808080808080808080808080808080808080808080808080808080808"))))]
    #[sov_wallet(display(hex))]
    hex_address: [u8; 32],
    #[sov_wallet(template("transfer" = input("to_bech32"), "receive" = value(bytes("celestia1pqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyq3k0ptp"))))]
    #[sov_wallet(display(bech32(prefix = "PREFIX_CELESTIA")))]
    bech32_address: [u8; 32],
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub enum SimpleEnumWithTemplate {
    #[sov_wallet(template("transfer_2"))]
    One(SimpleStructWithTemplate),
    #[sov_wallet(template("mint_2"))]
    Two {
        #[sov_wallet(template("mint_2" = input))]
        msg: u8,
    },
    #[sov_wallet(template("mint"))]
    Three(NestedStructWithNonNestedTemplates),
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
#[sov_wallet(template_inherit)]
pub enum SimplerEnumWithTemplate {
    One(SimpleStructWithTemplate),
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub enum SimpleEnumWithTemplateOverrides {
    #[sov_wallet(template_override_ty = "SurrogateSimpleStructWithTemplate")]
    #[sov_wallet(template("mint_a"))]
    One(SimpleStructWithTemplate),
    #[sov_wallet(template_override_ty = "()")]
    Two {
        #[sov_wallet(template("mint_2" = input("mint_msg")))]
        msg: u8,
    },
    Three(NestedStructWithNonNestedTemplates),
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct NestedStructWithTemplate {
    inner: SimpleStructWithTemplate,
    #[sov_wallet(template("transfer" = value("6"), "transfer_2" = input("int_msg")))]
    msg: u8,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct NestedStructWithTemplateOverride {
    #[sov_wallet(template_override_ty = "()")]
    inner: SimpleStructWithTemplate,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct SurrogateSimpleStructWithTemplate {
    #[sov_wallet(template("mint_a" = value("19")))]
    tokens: u64,
    #[sov_wallet(template("mint_a" = input("mint_msg")))]
    msg: SafeString,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct NestedStructWithSurrogateTemplateOverride {
    #[sov_wallet(template_override_ty = "SurrogateSimpleStructWithTemplate")]
    inner: SimpleStructWithTemplate,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Clone,
    UniversalWallet,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct NestedStructWithNonNestedTemplates {
    #[sov_wallet(template("mint" = value("4,aa")))]
    inner: SimpleStructWithTemplate,
    #[sov_wallet(template("mint" = input("inner")))]
    inner2: SimpleStructWithTemplate,
    #[sov_wallet(template("mint" = input("top_msg")))]
    msg: u32,
}

#[test]
fn test_simple_struct_schema_with_template() {
    let my_registration = SimpleStructWithTemplate {
        tokens: 1000,
        msg: "abc".to_string().try_into().unwrap(),
    };

    encode_decode_tests!(
        SimpleStructWithTemplate,
        my_registration,
        "{ tokens: 1000, msg: \"abc\" }"
    );

    let schema = Schema::of_single_type::<SimpleStructWithTemplate>().unwrap();
    assert_eq!(schema.templates(0).unwrap(), vec!["transfer", "transfer_2"]);

    let transfer_example_encoding = borsh::to_vec(&SimpleStructWithTemplate {
        tokens: 124,
        msg: "ababab".to_string().try_into().unwrap(),
    })
    .unwrap();
    let transfer_template_encoding = schema
        .fill_template_from_json(0, "transfer", "{ \"amount\": 124 }")
        .unwrap();
    assert_eq!(transfer_example_encoding, transfer_template_encoding);

    let transfer_2_example_encoding = borsh::to_vec(&SimpleStructWithTemplate {
        tokens: Default::default(),
        msg: "aaabb".to_string().try_into().unwrap(),
    })
    .unwrap();
    let transfer_2_template_encoding = schema
        .fill_template_from_json(0, "transfer_2", "{ \"msg\": \"aaabb\" }")
        .unwrap();
    assert_eq!(transfer_2_example_encoding, transfer_2_template_encoding);
}

#[test]
fn test_nested_struct_with_template_override() {
    let schema = Schema::of_single_type::<NestedStructWithTemplateOverride>().unwrap();
    assert!(schema.templates(0).unwrap().is_empty());
}

#[test]
fn test_nested_struct_with_surrogate_templates() {
    let my_struct = NestedStructWithSurrogateTemplateOverride {
        inner: SimpleStructWithTemplate {
            tokens: 1000,
            msg: "abc".to_string().try_into().unwrap(),
        },
    };

    encode_decode_tests!(
        NestedStructWithSurrogateTemplateOverride,
        my_struct,
        "{ inner: { tokens: 1000, msg: \"abc\" } }"
    );

    let schema = Schema::of_single_type::<NestedStructWithSurrogateTemplateOverride>().unwrap();
    assert_eq!(schema.templates(0).unwrap(), vec!["mint_a"]);

    let mint_example_encoding = borsh::to_vec(&NestedStructWithSurrogateTemplateOverride {
        inner: SimpleStructWithTemplate {
            tokens: 19,
            msg: "hi".to_string().try_into().unwrap(),
        },
    })
    .unwrap();
    let mint_template_encoding = schema
        .fill_template_from_json(0, "mint_a", "{ \"mint_msg\": \"hi\" }")
        .unwrap();
    assert_eq!(mint_example_encoding, mint_template_encoding);
}

#[test]
fn test_simple_struct_schema_with_template_and_display() {
    let my_registration = SimpleStructWithTemplateAndDisplays {
        tokens: 1000,
        hex_address: [8; 32],
        bech32_address: [9; 32],
    };

    encode_decode_tests!(
        SimpleStructWithTemplateAndDisplays,
        my_registration,
        "{ tokens: 1000, hex_address: 0x0808080808080808080808080808080808080808080808080808080808080808, bech32_address: celestia1pyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyys5ykmar }"
    );

    let schema = Schema::of_single_type::<SimpleStructWithTemplateAndDisplays>().unwrap();

    let transfer_example_encoding = borsh::to_vec(&SimpleStructWithTemplateAndDisplays {
        tokens: 124,
        hex_address: [9; 32],
        bech32_address: [9; 32],
    })
    .unwrap();
    let transfer_template_encoding = schema
        .fill_template_from_json(0, "transfer", "{ \"tokens\": 124, \"to_hex\": \"0x0909090909090909090909090909090909090909090909090909090909090909\", \"to_bech32\": \"celestia1pyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyys5ykmar\" }")
        .unwrap();
    assert_eq!(transfer_example_encoding, transfer_template_encoding);

    let receive_example_encoding = borsh::to_vec(&SimpleStructWithTemplateAndDisplays {
        tokens: 125,
        hex_address: [8; 32],
        bech32_address: [8; 32],
    })
    .unwrap();
    let receive_template_encoding = schema
        .fill_template_from_json(0, "receive", "{ \"tokens\": 125 }")
        .unwrap();
    assert_eq!(receive_example_encoding, receive_template_encoding);
}

#[test]
fn test_simple_enum_schema_with_template() {
    let my_registration = SimpleEnumWithTemplate::One(SimpleStructWithTemplate {
        tokens: 1000,
        msg: "abc".to_string().try_into().unwrap(),
    });

    encode_decode_tests!(
        SimpleEnumWithTemplate,
        my_registration,
        "One { tokens: 1000, msg: \"abc\" }"
    );

    let schema = Schema::of_single_type::<SimpleEnumWithTemplate>().unwrap();
    assert_eq!(
        schema.templates(0).unwrap(),
        vec!["mint", "mint_2", "transfer_2"]
    );

    let variant_one_encoding =
        borsh::to_vec(&SimpleEnumWithTemplate::One(SimpleStructWithTemplate {
            tokens: 0,
            msg: "bbb".to_string().try_into().unwrap(),
        }))
        .unwrap();
    let variant_one_template_encoding = schema
        .fill_template_from_json(0, "transfer_2", "{ \"msg\": \"bbb\" }")
        .unwrap();
    assert_eq!(variant_one_encoding, variant_one_template_encoding);

    let variant_two_encoding = borsh::to_vec(&SimpleEnumWithTemplate::Two { msg: 9 }).unwrap();
    let variant_two_template_encoding = schema
        .fill_template_from_json(0, "mint_2", "{ \"msg\": 9 }")
        .unwrap();
    assert_eq!(variant_two_encoding, variant_two_template_encoding);

    let variant_three_encoding = borsh::to_vec(&SimpleEnumWithTemplate::Three(
        NestedStructWithNonNestedTemplates {
            inner: SimpleStructWithTemplate {
                tokens: 4,
                msg: "aa".try_into().unwrap(),
            },
            inner2: SimpleStructWithTemplate {
                msg: "ababa".try_into().unwrap(),
                tokens: 1344,
            },
            msg: 43,
        },
    ))
    .unwrap();
    let variant_three_template_encoding = schema
        .fill_template_from_json(
            0,
            "mint",
            "{ \"top_msg\": 43, \"inner\": { \"msg\": \"ababa\", \"tokens\": 1344 } }",
        )
        .unwrap();
    assert_eq!(variant_three_encoding, variant_three_template_encoding);
}

#[test]
fn test_nested_struct_schema_with_template() {
    let my_registration = NestedStructWithTemplate {
        inner: SimpleStructWithTemplate {
            tokens: 1000,
            msg: "abc".to_string().try_into().unwrap(),
        },
        msg: 19,
    };

    encode_decode_tests!(
        NestedStructWithTemplate,
        my_registration,
        "{ inner: { tokens: 1000, msg: \"abc\" }, msg: 19 }"
    );

    let schema = Schema::of_single_type::<NestedStructWithTemplate>().unwrap();
    assert_eq!(schema.templates(0).unwrap(), vec!["transfer", "transfer_2"]);

    let transfer_example_encoding = borsh::to_vec(&NestedStructWithTemplate {
        inner: SimpleStructWithTemplate {
            tokens: 124,
            msg: "ababab".try_into().unwrap(),
        },
        msg: 6,
    })
    .unwrap();
    let transfer_template_encoding = schema
        .fill_template_from_json(0, "transfer", "{ \"amount\": 124 }")
        .unwrap();
    assert_eq!(transfer_example_encoding, transfer_template_encoding);

    let transfer_2_example_encoding = borsh::to_vec(&NestedStructWithTemplate {
        inner: SimpleStructWithTemplate {
            tokens: 0,
            msg: "two".try_into().unwrap(),
        },
        msg: 93,
    })
    .unwrap();
    let transfer_2_template_encoding = schema
        .fill_template_from_json(0, "transfer_2", "{ \"msg\": \"two\", \"int_msg\": 93 }")
        .unwrap();
    assert_eq!(transfer_2_example_encoding, transfer_2_template_encoding);
}

#[test]
fn test_simple_enum_schema_with_template_overrides() {
    let schema = Schema::of_single_type::<SimpleEnumWithTemplateOverrides>().unwrap();
    assert_eq!(schema.templates(0).unwrap(), vec!["mint_a"]);
}

#[test]
fn test_nested_struct_schema_with_non_nested_template() {
    let my_registration = NestedStructWithNonNestedTemplates {
        inner: SimpleStructWithTemplate {
            tokens: 1000,
            msg: "abc".to_string().try_into().unwrap(),
        },
        inner2: SimpleStructWithTemplate {
            tokens: 1000,
            msg: "abc".to_string().try_into().unwrap(),
        },
        msg: 40203,
    };
    encode_decode_tests!(NestedStructWithNonNestedTemplates, my_registration, "{ inner: { tokens: 1000, msg: \"abc\" }, inner2: { tokens: 1000, msg: \"abc\" }, msg: 40203 }");

    let schema = Schema::of_single_type::<NestedStructWithNonNestedTemplates>().unwrap();
    assert_eq!(schema.templates(0).unwrap(), vec!["mint"]);
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum SimpleEnumWithShowAs {
    #[cfg_attr(
        test,
        sov_wallet(show_as = "This variant has {} tokens, and the following message: {}. End.")
    )]
    One { tokens: u64, msg: SafeString },
    #[cfg_attr(
        test,
        sov_wallet(show_as = "This variant is a tuple with two fields: a string {} an u8 {}.")
    )]
    Two(SafeString, u8),
}

#[test]
fn test_simple_enum_schema_with_showas() {
    let var_one = SimpleEnumWithShowAs::One {
        tokens: 1000,
        msg: "abc".to_string().try_into().unwrap(),
    };
    encode_decode_tests!(
        SimpleEnumWithShowAs,
        var_one,
        "This variant has 1000 tokens, and the following message: \"abc\". End."
    );

    let var_two = SimpleEnumWithShowAs::Two("def".to_string().try_into().unwrap(), 19);
    encode_decode_tests!(
        SimpleEnumWithShowAs,
        var_two,
        "This variant is a tuple with two fields: a string \"def\" an u8 19."
    );
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[serde(rename_all = "snake_case")]
pub struct Registration {
    address: [u8; 32],
    role: Role,
    tokens: u64,
    #[cfg_attr(test, sov_wallet(as_ty = "u64"))]
    schemaless: NoSchemaU64Wrapper,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct StringWrapper(pub SafeString);

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(BorshSerialize, BorshDeserialize))]
pub struct SchemalessStringWrapper(pub SafeString);

#[cfg(test)]
impl UniversalWallet for SchemalessStringWrapper {
    fn scaffold() -> Item<IndexLinking> {
        Item::Atom(Primitive::String)
    }
    fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
        Vec::new()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, UniversalWallet)]
#[cfg_attr(test, derive(BorshSerialize, BorshDeserialize))]
pub struct VecOfThing {
    memo: Vec<StringWrapper>,
}

#[derive(Debug, PartialEq, Eq, Clone, UniversalWallet)]
#[cfg_attr(test, derive(BorshSerialize, BorshDeserialize))]
pub struct VecOfWrapper {
    memo: Vec<SchemalessStringWrapper>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct RegistrationLike {
    address: [u8; 32],
    some_bytes: Vec<u8>,
    extra_complexity: Vec<AThirdComplexType>,
}

type VecAlias = Vec<u8>;
type IntAlias = i32;
type TestCallAlias = TestCall;

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
#[serde(rename_all = "snake_case")]
pub enum TestCallRec<T> {
    Withdraw(Box<T>),
    // #[serde(skip)]
    #[allow(unused)]
    // We need this variant to test the schema generation of recursive types even though we don't construct it
    Complex(#[sov_wallet(bound = "Box<T>: UniversalWallet")] Box<ComplexRec<T>>),
    // Complex(Box<ComplexRec<T>>),
}

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
#[serde(rename_all = "snake_case")]
pub enum TestCallStructRec<T> {
    Withdraw(Box<T>),
    #[allow(unused)]
    Complex {
        #[sov_wallet(bound = "Box<T>: UniversalWallet")]
        rec_field: Box<ComplexRec<T>>,
    },
}

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub struct ComplexRec<T> {
    unnested_enum: TestCallRec<T>,
}

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub struct VariousTypes {
    option: Option<MinimalStruct>,
    other_option: Option<u8>,
    range: Range<u8>,
    hash_map: HashMap<SafeString, u8>,
    enum_map: HashMap<Role, u8>,
    num_map: HashMap<u32, u8>,
}

#[test]
fn test_various_types() {
    let mut hash_map: HashMap<SafeString, u8> = HashMap::new();
    hash_map.insert("a".try_into().unwrap(), 9);
    hash_map.insert("b".try_into().unwrap(), 16);
    let mut num_map: HashMap<u32, u8> = HashMap::new();
    num_map.insert(10_000, 12);
    num_map.insert(20_000, 13);
    let mut enum_map: HashMap<Role, u8> = HashMap::new();
    enum_map.insert(Role::Attester, 9);
    enum_map.insert(Role::Challenger, 16);
    let my_types = VariousTypes {
        option: Some(MinimalStruct { tokens: 3 }),
        other_option: None,
        range: 4..8,
        hash_map,
        enum_map,
        num_map,
    };

    encode_decode_tests!(
        VariousTypes,
        my_types,
        "{ option: { tokens: 3 }, other_option: None, range: 4..8, hash_map: { \"a\": 9, \"b\": 16 }, enum_map: { .Attester: 9, .Challenger: 16 }, num_map: { 10000: 12, 20000: 13 } }"
    );
}

#[test]
fn test_tuple_schema_recursive_generic() {
    let my_call = TestCallRec::<u64>::Withdraw(Box::new(43));

    encode_decode_tests!(TestCallRec<u64>, my_call, "Withdraw(43)");
}

#[test]
fn test_tuple_struct_schema_recursive_generic() {
    let my_call = TestCallStructRec::<u64>::Withdraw(Box::new(43));

    encode_decode_tests!(TestCallStructRec<u64>, my_call, "Withdraw(43)");
}

#[serde_as]
#[derive(Debug, PartialEq, Serialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub struct StructWithPrimitives {
    pub u8: u8,
    #[serde_as(as = "DisplayFromStr")]
    pub u8_str: u8,
    pub u16: u16,
    #[serde_as(as = "DisplayFromStr")]
    pub u16_str: u16,
    pub u32: u32,
    #[serde_as(as = "DisplayFromStr")]
    pub u32_str: u32,
    pub u64: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub u64_str: u64,
    // No native u128 because it doesn't fit in JSON numbers
    #[serde_as(as = "DisplayFromStr")]
    pub u128: u128,
    pub i8: i8,
    #[serde_as(as = "DisplayFromStr")]
    pub i8_str: i8,
    pub i16: i16,
    #[serde_as(as = "DisplayFromStr")]
    pub i16_str: i16,
    pub i32: i32,
    #[serde_as(as = "DisplayFromStr")]
    pub i32_str: i32,
    pub i64: i64,
    #[serde_as(as = "DisplayFromStr")]
    pub i64_str: i64,
    #[serde_as(as = "DisplayFromStr")]
    pub i128: i128,
    pub bool: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub bool_str: bool,
    pub f32: f32,
    #[serde_as(as = "DisplayFromStr")]
    pub f32_str: f32,
    pub f64: f64,
    #[serde_as(as = "DisplayFromStr")]
    pub f64_str: f64,
    pub string: SafeString,
}

#[test]
fn test_struct_with_primitives() {
    let my_struct = StructWithPrimitives {
        u8: 92,
        u8_str: 82,
        u16: 392,
        u16_str: 492,
        u32: 15_472_432,
        u32_str: 25_472_432,
        u64: 340_542_814_143,
        u64_str: 240_542_814_143,
        u128: 180_446_744_073_709_551_615,
        i8: -92,
        i8_str: -82,
        i16: -392,
        i16_str: -492,
        i32: -15_472_432,
        i32_str: -25_472_432,
        i64: -340_542_814_143,
        i64_str: -240_542_814_143,
        i128: -180_446_744_073_709_551_615,
        bool: true,
        bool_str: false,
        f32: 45.59,
        f32_str: 35.59,
        f64: 9716235.31632546,
        f64_str: 8716235.31632546,
        string: "Hello".to_string().try_into().unwrap(),
    };

    encode_decode_tests!(StructWithPrimitives, my_struct, "{ u8: 92, u8_str: 82, u16: 392, u16_str: 492, u32: 15472432, u32_str: 25472432, u64: 340542814143, u64_str: 240542814143, u128: 180446744073709551615, i8: -92, i8_str: -82, i16: -392, i16_str: -492, i32: -15472432, i32_str: -25472432, i64: -340542814143, i64_str: -240542814143, i128: -180446744073709551615, bool: true, bool_str: false, f32: 45.59, f32_str: 35.59, f64: 9716235.31632546, f64_str: 8716235.31632546, string: \"Hello\" }");
}

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub enum TestCallRecNonGeneric {
    Withdraw(u64),
    // #[serde(skip)]
    #[allow(unused)]
    // We need this variant to test the schema generation of recursive types even though we don't construct it
    Complex(#[sov_wallet(bound = "")] Box<ComplexRecNonGeneric>),
}
#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub struct ComplexRecNonGeneric {
    unnested_enum: TestCallRecNonGeneric,
}
#[test]
fn test_medium_struct_schema_recursive_nongeneric() {
    let my_call = TestCallRecNonGeneric::Withdraw(43);

    encode_decode_tests!(TestCallRecNonGeneric, my_call, "Withdraw(43)");
}

const PREFIX_SOV: &str = "sov";
const PREFIX_CELESTIA: &str = "celestia";

#[derive(Debug, PartialEq, Serialize, Eq, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct Complex {
    #[cfg_attr(test, sov_wallet(display(bech32(prefix = "PREFIX_SOV"))))]
    rollup_address: [u8; 32],
    #[cfg_attr(
        test,
        sov_wallet(display(bech32m(prefix = "PREFIX_CELESTIA"))),
        serde(with = "serde_arrays")
    )]
    da_address: [u8; 64],
    #[cfg_attr(test, sov_wallet(display(decimal)))]
    message: Vec<u8>,
    // Important: we cover the case of...
    // - an aliased field, followed by a field with a different schema, followed by a Vec containing the aliased type
    // using SchemalessStringWrapper. Be careful about reording and/or deleting the aliased field here, since
    // we can have regressions on that edge case.
    first_item: SchemalessStringWrapper,
    role: Role,
    tokens: u64,
    memo: Vec<SchemalessStringWrapper>,
    registration: Registration,
    events: Vec<Vec<u8>>,
    child_addresses: Vec<[u8; 32]>,
    nested_vec_bytes: Vec<Vec<Vec<u8>>>,
    nested_vec_enum: Vec<Vec<TestCall>>,
    aliased_vec: VecAlias,
    aliased_int: IntAlias,
    aliased_call: TestCallAlias,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct Generic<T> {
    contents: T,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct NestedGeneric<T> {
    contents: Generic<T>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct StructWithIntegerDisplays {
    #[cfg_attr(test, sov_wallet(fixed_point(2)))]
    direct_fp: u64,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(0))))]
    from_field_before: i64,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(4, offset = 4))))]
    from_field_after: u64,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(3))))]
    from_field_self: u64,
    #[cfg_attr(test, sov_wallet(hidden))]
    array_field: [u8; 5],
}

#[test]
fn test_struct_with_integer_fixedpoints() {
    let my_struct = StructWithIntegerDisplays {
        direct_fp: 4,
        from_field_before: -21,
        from_field_after: 4000,
        from_field_self: 3,
        array_field: [3, 3, 3, 3, 2],
    };

    encode_decode_tests!(StructWithIntegerDisplays, my_struct,
        "{ direct_fp: 0.04, from_field_before: -0.0021, from_field_after: 40, from_field_self: 0.003 }"
    );
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct TupleWithIntegerDisplaysAndNesting(
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(5, offset = 1))))] u16,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(0))))] i8,
    StructWithIntegerDisplays,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(4))))] u128,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(4))))] u8,
    #[cfg_attr(test, sov_wallet(hidden))] [u8; 2],
);

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct StructWithIntegerDisplaysAndNesting {
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(0))))]
    from_field_self: u64,
    nested_tuple: TupleWithIntegerDisplaysAndNesting,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(3))))]
    from_field_after: u128,
    #[cfg_attr(test, sov_wallet(fixed_point(from_field(0))))]
    from_field_before: u64,
}

#[test]
fn test_struct_with_integer_fixedpoints_and_nesting() {
    let my_struct = StructWithIntegerDisplaysAndNesting {
        from_field_self: 2,
        nested_tuple: TupleWithIntegerDisplaysAndNesting(
            2,
            -21,
            StructWithIntegerDisplays {
                direct_fp: 4,
                from_field_before: -21,
                from_field_after: 4000,
                from_field_self: 3,
                array_field: [3, 3, 3, 3, 4],
            },
            475000,
            4,
            [19, 2],
        ),
        from_field_after: 40_000_000_000_000_000, // 40
        from_field_before: 15,
    };

    encode_decode_tests!(StructWithIntegerDisplaysAndNesting, my_struct,
        "{ from_field_self: 0.02, nested_tuple: (0.02, -0.21, { direct_fp: 0.04, from_field_before: -0.0021, from_field_after: 0.4, from_field_self: 0.003 }, 47.5, 0.0004), from_field_after: 40, from_field_before: 0.15 }"
    );
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct StructWithBase58 {
    #[cfg_attr(test, sov_wallet(display(base58)))]
    address: [u8; 32],
    #[cfg_attr(test, sov_wallet(display(base58)))]
    extra_bytes: Vec<u8>,
    role: Role,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub struct AThirdComplexType {
    #[cfg_attr(test, sov_wallet(display(bech32(prefix = "PREFIX_CELESTIA"))))]
    address: [u8; 32],
    #[cfg_attr(test, sov_wallet(display(decimal)))]
    extra_bytes: Vec<u8>,
    role: Role,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Clone,
    Serialize,
    Deserialize,
    BorshDeserialize,
    BorshSerialize,
    UniversalWallet,
)]
pub enum Role {
    Attester,
    Challenger,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
#[serde(rename_all = "snake_case", rename = "a")]
pub enum RuntimeCall {
    TestCall(TestCall),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithStruct {
    Foo {
        first_field: u64,
        second_field: SafeString,
        third_field: u32,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithTwoStructs {
    Foo {
        first_field: u64,
        second_field: SafeString,
        third_field: u32,
    },
    Bar {
        first_field: u64,
        second_field: u8,
        third_field: u32,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithStructAndGeneric<T> {
    Foo {
        first_field: u64,
        second_field: Generic<T>,
        third_field: u32,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithStructAndThreeGenerics<T, U, V> {
    VarA {
        first_field: u64,
        second_field: Generic<T>,
        third_field: u32,
    },
    VarB {
        first_field: u64,
        second_field: Generic<U>,
        third_field: u32,
    },
    VarC {
        first_field: u64,
        second_field: Generic<Generic<V>>,
        third_field: u32,
    },
    VarD,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithMultiTupleSimple {
    TheVariant(u64, EnumWithStruct, SafeString),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithMultiTuple {
    TheVariant(u64, EnumWithStruct, RuntimeCall),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithMultiTupleAndGenerics<T, U, V> {
    One(u64, EnumWithStructAndThreeGenerics<T, U, V>, SafeString),
    Two(Generic<U>),
    Three,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize, BorshDeserialize))]
pub enum EnumWithIdenticalTuples {
    FirstVariant(u64, EnumWithStruct, SafeString),
    SecondVariant(u64, EnumWithStruct, SafeString),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
pub struct WithSilentField<T> {
    int: u64,
    #[cfg_attr(test, sov_wallet(hidden))]
    skipped: T,
    str: SafeString,
    phantom: PhantomData<u64>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[cfg_attr(test, derive(UniversalWallet, BorshSerialize))]
struct WithTuples {
    #[allow(unused_parens)]
    double: (u64, u64),
    mixed: (u64, SafeString, Role),
    quintuple: (u64, u64, u64, u64, u64),
    octuple: (u64, u64, u64, u64, u64, u64, u64, u64),
}

#[test]
fn test_tuples() {
    let my_with_tuples = WithTuples {
        double: (5, 8),
        mixed: (
            13214,
            "hello".to_string().try_into().unwrap(),
            Role::Challenger,
        ),
        quintuple: (1, 2, 3, 4, 5),
        octuple: (1, 2, 3, 4, 5, 6, 7, 8),
    };

    encode_decode_tests!(WithTuples, my_with_tuples, "{ double: (5, 8), mixed: (13214, \"hello\", RoleChallenger), quintuple: (1, 2, 3, 4, 5), octuple: (1, 2, 3, 4, 5, 6, 7, 8) }");
}

#[test]
fn test_enum_with_complex_tuples() {
    let runtime_call = RuntimeCall::TestCall(TestCall::Register(Registration {
        address: [17; 32],
        role: Role::Attester,
        tokens: 1000,
        schemaless: NoSchemaU64Wrapper(123),
    }));

    let my_enum = EnumWithMultiTuple::TheVariant(
        16,
        EnumWithStruct::Foo {
            first_field: 84,
            second_field: "abcd".to_string().try_into().unwrap(),
            third_field: 14,
        },
        runtime_call,
    );

    encode_decode_tests!(EnumWithMultiTuple, my_enum,
        "TheVariant(16, Foo { first_field: 84, second_field: \"abcd\", third_field: 14 }, TestCall.Register { address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 })"
    );
}

#[test]
fn test_enum_with_simple_tuples() {
    let my_enum = EnumWithMultiTupleSimple::TheVariant(
        16,
        EnumWithStruct::Foo {
            first_field: 84,
            second_field: "abcd".to_string().try_into().unwrap(),
            third_field: 14,
        },
        "hello".to_string().try_into().unwrap(),
    );

    encode_decode_tests!(EnumWithMultiTupleSimple, my_enum,
        "TheVariant(16, Foo { first_field: 84, second_field: \"abcd\", third_field: 14 }, \"hello\")"
    );
}

#[test]
fn test_enum_with_identical_tuples() {
    let my_enum = EnumWithIdenticalTuples::SecondVariant(
        16,
        EnumWithStruct::Foo {
            first_field: 84,
            second_field: "abcd".to_string().try_into().unwrap(),
            third_field: 14,
        },
        "hello".to_string().try_into().unwrap(),
    );

    encode_decode_tests!(EnumWithIdenticalTuples, my_enum,
        "SecondVariant(16, Foo { first_field: 84, second_field: \"abcd\", third_field: 14 }, \"hello\")"
    );
}

#[test]
fn test_enum_with_struct() {
    let my_with_tuples = EnumWithStruct::Foo {
        first_field: 84,
        second_field: "abcd".to_string().try_into().unwrap(),
        third_field: 14,
    };

    encode_decode_tests!(
        EnumWithStruct,
        my_with_tuples,
        "Foo { first_field: 84, second_field: \"abcd\", third_field: 14 }"
    );
}

#[test]
fn test_enum_with_struct_and_generic() {
    let my_with_tuples = EnumWithStructAndGeneric::Foo {
        first_field: 84,
        second_field: Generic { contents: 52 },
        third_field: 14,
    };

    encode_decode_tests!(
        EnumWithStructAndGeneric<u32>,
        my_with_tuples,
        "Foo { first_field: 84, second_field: { contents: 52 }, third_field: 14 }"
    );
}

#[test]
fn test_enum_with_struct_and_multiple_generics() {
    let my_with_generics: EnumWithStructAndThreeGenerics<u32, u8, i8> =
        EnumWithStructAndThreeGenerics::VarA {
            first_field: 84,
            second_field: Generic {
                contents: 2_000_000_000,
            },
            third_field: 14,
        };

    encode_decode_tests!(
        EnumWithStructAndThreeGenerics<u32, u8, i8>,
        my_with_generics,
        "VarA { first_field: 84, second_field: { contents: 2000000000 }, third_field: 14 }"
    );
}

#[test]
fn test_enum_with_tuples_and_generics() {
    let my_var_two: EnumWithMultiTupleAndGenerics<u32, u8, i8> =
        EnumWithMultiTupleAndGenerics::Two(Generic { contents: 19 });
    let my_var_one: EnumWithMultiTupleAndGenerics<u32, u8, i8> = EnumWithMultiTupleAndGenerics::One(
        1245345,
        EnumWithStructAndThreeGenerics::VarC {
            first_field: 435653,
            second_field: Generic {
                contents: Generic { contents: -5 },
            },
            third_field: 73242,
        },
        "abdsf".to_string().try_into().unwrap(),
    );

    encode_decode_tests!(EnumWithMultiTupleAndGenerics<u32, u8, i8>, my_var_two, "Two { contents: 19 }");

    encode_decode_tests!(EnumWithMultiTupleAndGenerics<u32, u8, i8>, my_var_one,
    "One(1245345, VarC { first_field: 435653, second_field: { contents: { contents: -5 } }, third_field: 73242 }, \"abdsf\")"
    );
}

#[test]
fn test_minimal_enum_schema() {
    let item = Role::Attester;
    encode_decode_tests!(Role, item, "Attester");
}

#[test]
fn test_minimal_struct_schema() {
    let my_registration = MinimalStruct { tokens: 1000 };

    encode_decode_tests!(MinimalStruct, my_registration, "{ tokens: 1000 }");
}

#[test]
fn test_simple_struct_schema() {
    let my_registration = Registration {
        address: [17; 32],
        role: Role::Attester,
        tokens: 1000,
        schemaless: NoSchemaU64Wrapper(123),
    };

    encode_decode_tests!(Registration, my_registration, "{ address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 }");
}

#[test]
fn test_multiobject_schema() {
    let schema =
        Schema::of_rollup_types_with_chain_data::<Role, MinimalStruct, Registration, SimpleEnum>(
            ChainData {
                chain_id: 4321,
                chain_name: "Testchain".to_string(),
            },
        )
        .unwrap();

    let my_role = Role::Attester;
    let my_minimal_struct = MinimalStruct { tokens: 1000 };
    let my_registration = Registration {
        address: [17; 32],
        role: Role::Attester,
        tokens: 1000,
        schemaless: NoSchemaU64Wrapper(123),
    };

    let orig_hash = schema.cached_chain_hash().unwrap();
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();
    let mut schema = Schema::from_json(&schema_json).unwrap();
    let hash = schema.chain_hash().unwrap();
    assert_eq!(orig_hash, hash);

    // TODO: ugly
    let role_borsh_ser = borsh::to_vec(&my_role).unwrap();
    let role_json = serde_json::to_string(&my_role).unwrap();
    assert_eq!(
        schema
            .display(
                schema
                    .rollup_expected_index(RollupRoots::Transaction)
                    .unwrap(),
                &role_borsh_ser
            )
            .unwrap(),
        "Attester"
    );
    assert_eq!(
        schema
            .json_to_borsh(
                schema
                    .rollup_expected_index(RollupRoots::Transaction)
                    .unwrap(),
                &role_json
            )
            .unwrap(),
        role_borsh_ser
    );
    let struct_borsh_ser = borsh::to_vec(&my_minimal_struct).unwrap();
    let struct_json = serde_json::to_string(&my_minimal_struct).unwrap();
    assert_eq!(
        schema
            .display(
                schema
                    .rollup_expected_index(RollupRoots::UnsignedTransaction)
                    .unwrap(),
                &struct_borsh_ser
            )
            .unwrap(),
        "{ tokens: 1000 }"
    );
    assert_eq!(
        schema
            .json_to_borsh(
                schema
                    .rollup_expected_index(RollupRoots::UnsignedTransaction)
                    .unwrap(),
                &struct_json
            )
            .unwrap(),
        struct_borsh_ser
    );
    let reg_borsh_ser = borsh::to_vec(&my_registration).unwrap();
    let reg_json = serde_json::to_string(&my_registration).unwrap();
    assert_eq!(schema.display(schema.rollup_expected_index(RollupRoots::RuntimeCall).unwrap(), &reg_borsh_ser).unwrap(), "{ address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 }");
    assert_eq!(
        schema
            .json_to_borsh(
                schema
                    .rollup_expected_index(RollupRoots::RuntimeCall)
                    .unwrap(),
                &reg_json
            )
            .unwrap(),
        reg_borsh_ser
    );
}

#[test]
fn test_medium_struct_schema() {
    let my_call = RuntimeCall::TestCall(TestCall::Register(Registration {
        address: [17; 32],
        role: Role::Attester,
        tokens: 1000,
        schemaless: NoSchemaU64Wrapper(123),
    }));

    encode_decode_tests!(RuntimeCall, my_call, "TestCall.Register { address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 }");
}

#[test]
fn test_vec_schema() {
    let my_call = RuntimeCall::TestCall(TestCall::RegisterMany(vec![RegistrationLike {
        address: [23; 32],
        some_bytes: vec![1, 2, 3, 4, 5],
        extra_complexity: vec![AThirdComplexType {
            address: [17; 32],
            extra_bytes: vec![6, 7, 8, 9, 10],
            role: Role::Attester,
        }],
    }]));

    encode_decode_tests!(RuntimeCall, my_call,
        "TestCall.RegisterMany [{ address: 0x1717171717171717171717171717171717171717171717171717171717171717, some_bytes: 0x0102030405, extra_complexity: [{ address: celestia1zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygsr0ealj, extra_bytes: [6, 7, 8, 9, 10], role: Attester }] }]"
    );
}

#[test]
fn test_base58() {
    let my_call = StructWithBase58 {
        address: [17; 32],
        extra_bytes: vec![6, 7, 8, 9, 10],
        role: Role::Attester,
    };

    encode_decode_tests!(StructWithBase58, my_call,
        "{ address: 29d2S7vB453rNYFdR5Ycwt7y9haRT5fwVwL9zTmBhfV2, extra_bytes: gScbNR, role: Attester }"
    );
}

#[test]
fn test_vec_simple_schema() {
    let my_call = RuntimeCall::TestCall(TestCall::RegisterSimple(vec![1, 2, 3]));

    encode_decode_tests!(RuntimeCall, my_call, "TestCall.RegisterSimple(0x010203)");
}

#[test]
fn test_vec_string() {
    let my_call = vec![
        StringWrapper("hello".to_string().try_into().unwrap()),
        StringWrapper("world".to_string().try_into().unwrap()),
    ];

    encode_decode_tests!(Vec<StringWrapper>, my_call, r#"["hello", "world"]"#);
}

#[test]
fn test_complex_type() {
    let my_call = Complex {
        rollup_address: [1; 32],
        da_address: [2; 64],
        message: vec![3, 2, 1],
        first_item: SchemalessStringWrapper("hello".to_string().try_into().unwrap()),
        role: Role::Attester,
        tokens: 1000,
        memo: vec![SchemalessStringWrapper(
            "This is a memo".to_string().try_into().unwrap(),
        )],
        registration: Registration {
            address: [17; 32],
            role: Role::Attester,
            tokens: 1000,
            schemaless: NoSchemaU64Wrapper(123),
        },
        events: vec![b"hello".to_vec(), b"goodbye".to_vec()],
        child_addresses: vec![[3; 32], [4; 32]],
        nested_vec_bytes: vec![vec![vec![0, 1, 2]]],
        nested_vec_enum: vec![vec![TestCall::Withdraw(1)], vec![TestCall::Withdraw(2)]],
        aliased_vec: vec![7, 8, 9],
        aliased_int: 1234567,
        aliased_call: TestCall::Register(Registration {
            address: [17; 32],
            role: Role::Attester,
            tokens: 1000,
            schemaless: NoSchemaU64Wrapper(123),
        }),
    };

    encode_decode_tests!(Complex, my_call,
        "{ rollup_address: sov1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqslg48nn, da_address: celestia1qgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs08sssm, message: [3, 2, 1], first_item: \"hello\", role: Attester, tokens: 1000, memo: [\"This is a memo\"], registration: { address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 }, events: [0x68656c6c6f, 0x676f6f64627965], child_addresses: [0x0303030303030303030303030303030303030303030303030303030303030303, 0x0404040404040404040404040404040404040404040404040404040404040404], nested_vec_bytes: [[0x000102]], nested_vec_enum: [[TestCall.Withdraw(1)], [TestCall.Withdraw(2)]], aliased_vec: 0x070809, aliased_int: 1234567, aliased_call: Register { address: 0x1111111111111111111111111111111111111111111111111111111111111111, role: Attester, tokens: 1000, schemaless: 123 } }"
    );
}

#[test]
fn test_silent_simple_field() {
    let my_call: WithSilentField<SafeString> = WithSilentField {
        int: 123,
        skipped: "this should be skipped".try_into().unwrap(),
        str: "this should be included".try_into().unwrap(),
        phantom: Default::default(),
    };

    encode_decode_tests!(
        WithSilentField<SafeString>,
        my_call,
        "{ int: 123, str: \"this should be included\" }"
    );
}
// TODO: Are Vec<Box<T>> Primitive when T: Primitive? I think so.
// What about Vec<Box<u8>>? Is that a bytevec? I think so - but how do we resolve it?

#[test]
fn test_primtive_indirection() {
    let generic: Generic<Box<Vec<u8>>> = Generic {
        contents: Box::new(vec![12, 34]),
    };

    encode_decode_tests!(Generic<Box<Vec<u8>>>, generic, "{ contents: 0x0c22 }");
}

#[test]
fn test_nested_generics() {
    let generic: NestedGeneric<Vec<u8>> = NestedGeneric {
        contents: Generic {
            contents: vec![8, 34],
        },
    };

    encode_decode_tests!(
        NestedGeneric<Vec<u8>>,
        generic,
        "{ contents: { contents: 0x0822 } }"
    );
}

#[test]
fn test_nested_silent_fields() {
    let my_call = WithSilentField {
        int: 123,
        skipped: WithSilentField {
            int: 456,
            skipped: 789,
            str: "hi".try_into().unwrap(),
            phantom: Default::default(),
        },
        phantom: PhantomData,
        str: "this should be included".try_into().unwrap(),
    };

    encode_decode_tests!(
        WithSilentField<WithSilentField<i32>>,
        my_call,
        "{ int: 123, str: \"this should be included\" }"
    );
}
