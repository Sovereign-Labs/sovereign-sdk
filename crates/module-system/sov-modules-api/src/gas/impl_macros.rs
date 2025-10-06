// Implments the `GasArray` trait for the wrapper around [$u; $n] (example: GasUnit is [u64; 2])
macro_rules! impl_gas_array {
    ($t: ty, $n:expr, $u:ty) => {
        impl GasArray for $t {
            type Scalar = $u;
            // u64::ZERO would be better, but we have const and don't want third party crate.
            const ZEROED: Self = Self::from_primitive([<$u>::MIN; $n]);

            const MAX: Self = Self::from_primitive([<$u>::MAX; $n]);

            fn checked_sub(&self, rhs: &Self) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in self.value.iter().zip(rhs.value.as_slice()).enumerate() {
                    if let Some(res) = l.checked_sub(*r) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }

                Some(Self::from(output))
            }

            fn checked_scalar_product(&self, scalar: $u) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, v) in self.value.iter().enumerate() {
                    if let Some(res) = v.checked_mul(scalar) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }

                Some(Self::from(output))
            }

            fn dim_is_less_than(&self, rhs: &Self) -> bool {
                for (l, r) in self.value.iter().zip(rhs.value.as_slice()) {
                    if l >= r {
                        return false;
                    }
                }
                true
            }

            fn dim_is_less_or_eq(&self, rhs: &Self) -> bool {
                for (l, r) in self.value.iter().zip(rhs.value.as_slice()) {
                    if l > r {
                        return false;
                    }
                }
                true
            }

            fn calculate_min(lhs: &Self, rhs: &Self) -> Self {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in lhs.value.iter().zip(rhs.value.iter()).enumerate() {
                    output[i] = std::cmp::min(*l, *r);
                }
                Self::from_primitive(output)
            }

            fn scalar_division(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.checked_div(scalar).unwrap_or(<$u>::from(0u64)));
                self
            }

            #[cfg(feature = "test-utils")]
            fn scalar_add(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.saturating_add(scalar));
                self
            }

            #[cfg(feature = "test-utils")]
            fn scalar_sub(&mut self, scalar: $u) -> &mut Self {
                self.value
                    .iter_mut()
                    .for_each(|s| *s = s.saturating_sub(scalar));
                self
            }

            fn checked_combine(&self, rhs: &Self) -> Option<Self> {
                let mut output = [<$u>::from(0u64); $n];

                for (i, (l, r)) in self.value.iter().zip(rhs.value.iter()).enumerate() {
                    if let Some(res) = l.checked_add(*r) {
                        output[i] = res;
                    } else {
                        return None;
                    }
                }
                Some(Self::from_primitive(output))
            }
        }
    };
}

// Implement basic traits for wrappers around [$u; $n] (example: GasPrice is [u128; 2])
macro_rules! impl_gas_dimensions {
    ($t: ty, $t_name: literal, $n: expr, $u: ty) => {
        impl schemars::JsonSchema for $t {
            fn schema_name() -> String {
                $t_name.to_owned() + "(" + &format!("{}", $n) + ")"
            }

            fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
                serde_json::from_value(serde_json::json!({
                    "type": "array",
                    "minItems": $n,
                    "maxItems": $n,
                    "items": {
                        "type": "number"
                    },
                    // This description assumes that `serializer` uses a human-readable format.
                    "description": $t_name.to_owned() + " is an array of size " + &format!("{}", $n),
                }))
                .unwrap()
            }
        }

        impl From<[$u; $n]> for $t {
            fn from(array: [$u; $n]) -> Self {
                Self::from_primitive(array)
            }
        }

        impl TryFrom<Vec<$u>> for $t {
            type Error = anyhow::Error;

            fn try_from(value: Vec<$u>) -> Result<Self, Self::Error> {
                if value.len() != $n {
                    anyhow::bail!("Impossible to convert to a gas unit. The array must have {} elements, but it has {}", $n, value.len());
                }

                let mut output = [<$u>::from(0u64); $n];
                output.copy_from_slice(&value);

                Ok(Self::from(output))
            }
        }
    };
}

macro_rules! impl_serde {
    ($id: ident, $n:expr, $t: ty) => {
        impl ::serde::Serialize for $id<$n> {
            fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>
            where
                __S: serde::Serializer,
            {
                <[$t; $n] as serde::Serialize>::serialize(&self.value, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $id<$n> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let array = <[$t; $n] as serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::from(array))
            }
        }
    };
}

macro_rules! impl_gas_unit {
    ($n:expr) => {
        impl Gas for GasUnit<$n> {
            type Price = GasPrice<$n>;

            #[cfg(feature = "gas-constant-estimation")]
            fn name(&self) -> Option<&'static str> {
                self.name
            }

            /// Adds a name tag to the gas constant.
            #[cfg(feature = "gas-constant-estimation")]
            fn with_name(self, name: &'static str) -> Self {
                Self {
                    name: Some(name),
                    ..self
                }
            }

            fn checked_value(&self, price: &Self::Price) -> Option<Amount> {
                let mut value: Amount = Amount::ZERO;
                for (g, p) in self.value.iter().zip(price.as_ref().iter().copied()) {
                    let v = Amount::new(*g as u128).checked_mul(p)?;
                    value = value.checked_add(v)?;
                }

                Some(value)
            }

            fn value(&self, price: &Self::Price) -> Amount {
                self.value
                    .iter()
                    .zip(price.as_ref().iter().copied())
                    .map(|(a, b)| Amount::new(*a as u128).saturating_mul(b))
                    .fold(Amount::new(0), |a, b| a.saturating_add(b))
            }
        }

        impl GasUnit<$n> {
            /// Creates a new [`GasUnit`] from an array of [`u64`].
            const fn from_primitive(array: [u64; $n]) -> Self {
                Self {
                    value: array,
                    #[cfg(feature = "gas-constant-estimation")]
                    name: None,
                }
            }
        }

        impl GasPrice<$n> {
            /// Creates a new [`GasPrice`] from an array of Amount.
            #[must_use]
            pub const fn from_primitive(array: [Amount; $n]) -> Self {
                let mut value: [Amount; $n] = [Amount::ZERO; $n];

                let mut i = 0;
                while i < $n {
                    value[i] = array[i];
                    i += 1;
                }

                Self { value }
            }
        }

        impl_serde!(GasUnit, $n, u64);
        impl_gas_array!(GasUnit<$n>, $n, u64);
        impl_gas_dimensions!(GasUnit<$n>, "GasUnit", $n, u64);

        impl_serde!(GasPrice, $n, Amount);
        impl_gas_array!(GasPrice<$n>, $n, Amount);
        impl_gas_dimensions!(GasPrice<$n>, "GasPrice", $n, Amount);
    };
}

pub(crate) use impl_gas_array;
pub(crate) use impl_gas_dimensions;
pub(crate) use impl_gas_unit;
pub(crate) use impl_serde;
