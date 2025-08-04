use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sov_modules_api::macros::UniversalWallet;

const JS_MAX_SAFE_INTEGER: u128 = 9_007_199_254_740_991;
const JS_MIN_SAFE_INTEGER: i128 = -9_007_199_254_740_991;

macro_rules! impl_js_safe_unsigned {
    ($wrapper:ident, $inner:ty) => {
        #[derive(Debug, Arbitrary, BorshDeserialize, BorshSerialize, UniversalWallet)]
        pub struct $wrapper(pub $inner);

        impl Serialize for $wrapper {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let val = self.0 as u128;
                if val <= JS_MAX_SAFE_INTEGER {
                    serializer.serialize_u64(val as u64)
                } else {
                    serializer.serialize_str(&self.0.to_string())
                }
            }
        }

        impl<'de> Deserialize<'de> for $wrapper {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                use serde::de::Error;

                let value: Value = Deserialize::deserialize(deserializer)?;
                let num: u128 = match value {
                    Value::Number(n) => n
                        .as_u64()
                        .ok_or_else(|| D::Error::custom("Invalid number"))?
                        as u128,
                    Value::String(s) => s.parse().map_err(D::Error::custom)?,
                    _ => return Err(D::Error::custom("Expected number or string")),
                };

                if num > <$inner>::MAX as u128 {
                    return Err(D::Error::custom(format!(
                        "Value {} too large for {}",
                        num,
                        stringify!($inner)
                    )));
                }

                Ok($wrapper(num as $inner))
            }
        }
    };
}

impl_js_safe_unsigned!(JsU64, u64);
impl_js_safe_unsigned!(JsU128, u128);

macro_rules! impl_js_safe_signed {
    ($wrapper:ident, $inner:ty) => {
        #[derive(Debug, Arbitrary, BorshDeserialize, BorshSerialize, UniversalWallet)]
        pub struct $wrapper(pub $inner);

        impl Serialize for $wrapper {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let val = self.0 as i128;
                if val >= JS_MIN_SAFE_INTEGER && val <= JS_MAX_SAFE_INTEGER as i128 {
                    serializer.serialize_i64(val as i64)
                } else {
                    serializer.serialize_str(&self.0.to_string())
                }
            }
        }

        impl<'de> Deserialize<'de> for $wrapper {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                use serde::de::Error;

                let value: Value = Deserialize::deserialize(deserializer)?;
                let num: i128 = match value {
                    Value::Number(n) => n
                        .as_i64()
                        .ok_or_else(|| D::Error::custom("Invalid number"))?
                        as i128,
                    Value::String(s) => s.parse().map_err(D::Error::custom)?,
                    _ => return Err(D::Error::custom("Expected number or string")),
                };

                if num < <$inner>::MIN as i128 || num > <$inner>::MAX as i128 {
                    return Err(D::Error::custom(format!(
                        "Value {} out of range for {}",
                        num,
                        stringify!($inner)
                    )));
                }

                Ok($wrapper(num as $inner))
            }
        }
    };
}

impl_js_safe_signed!(JsI64, i64);
impl_js_safe_signed!(JsI128, i128);

// Serialize floats as hex encoded bytes to avoid precision loss when converting to/from JSON.
// Implements arbitrary that avoids NaN as this is not supported by borsh.
macro_rules! impl_js_safe_float {
    ($wrapper:ident, $inner:ty, $byte_count:expr) => {
        #[derive(Debug, BorshDeserialize, BorshSerialize, UniversalWallet)]
        pub struct $wrapper(pub $inner);

        impl Serialize for $wrapper {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let bytes = self.0.to_le_bytes();
                let hex_string = hex::encode(bytes);
                serializer.serialize_str(&format!("0x{}", hex_string))
            }
        }

        impl<'de> Deserialize<'de> for $wrapper {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                use serde::de::Error;

                let hex_string: String = Deserialize::deserialize(deserializer)?;
                let hex_string = hex_string.strip_prefix("0x").unwrap_or(&hex_string);
                let bytes = hex::decode(hex_string)
                    .map_err(|e| D::Error::custom(format!("Invalid hex string: {}", e)))?;

                if bytes.len() != $byte_count {
                    return Err(D::Error::custom(format!(
                        "Hex string must decode to exactly {} bytes for {}",
                        $byte_count,
                        stringify!($inner)
                    )));
                }

                let byte_array: [u8; $byte_count] = bytes
                    .try_into()
                    .map_err(|_| D::Error::custom("Failed to convert bytes to array"))?;

                Ok($wrapper(<$inner>::from_le_bytes(byte_array)))
            }
        }

        impl Arbitrary<'_> for $wrapper {
            fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
                let val: $inner = u.arbitrary()?;
                // borsh doesnt allow NaN
                let valid_val = if val.is_nan() { 0.0 } else { val };

                Ok($wrapper(valid_val))
            }
        }
    };
}

impl_js_safe_float!(JsF32, f32, 4);
impl_js_safe_float!(JsF64, f64, 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i128() {
        let large = JsI128(i128::MAX);
        let smaller = JsI128(JS_MIN_SAFE_INTEGER);

        assert_eq!(
            &serde_json::to_string(&large).unwrap(),
            "\"170141183460469231731687303715884105727\"" // string
        );
        assert_eq!(
            &serde_json::to_string(&smaller).unwrap(),
            "-9007199254740991" // int
        );
    }

    #[test]
    fn test_i64() {
        let large = JsI64(i64::MAX);
        let smaller = JsI64(-55i64);

        assert_eq!(
            &serde_json::to_string(&large).unwrap(),
            "\"9223372036854775807\""
        );
        assert_eq!(
            &serde_json::to_string(&smaller).unwrap(),
            "-55" // int
        );
    }

    #[test]
    fn test_u128() {
        let large = JsU128(u128::MAX);
        let smaller = JsU128(JS_MAX_SAFE_INTEGER);

        assert_eq!(
            &serde_json::to_string(&large).unwrap(),
            "\"340282366920938463463374607431768211455\""
        );
        assert_eq!(
            &serde_json::to_string(&smaller).unwrap(),
            "9007199254740991"
        );
    }

    #[test]
    fn test_u64() {
        let large = JsU64(u64::MAX);
        let smaller = JsU64(1234u64);

        assert_eq!(
            &serde_json::to_string(&large).unwrap(),
            "\"18446744073709551615\""
        );
        assert_eq!(
            &serde_json::to_string(&smaller).unwrap(),
            "1234" // int
        );
    }
}
