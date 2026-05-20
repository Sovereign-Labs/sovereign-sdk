use core::marker::PhantomData;

use sov_modules_api::{Spec, TxState};

use super::{Address, EvmPrecompileEnv, PrecompileResult};

/// A single read-only Sovereign EVM precompile.
pub trait EvmPrecompile<S: Spec>: Clone + Default + Send + Sync + 'static {
    /// The EVM address handled by this precompile.
    const ADDRESS: Address;

    /// Executes this precompile.
    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult;
}

/// A composable set of read-only Sovereign EVM precompiles.
pub trait EvmPrecompileSet<S: Spec>: Clone + Default + Send + Sync + 'static {
    /// The EVM addresses handled by this set.
    const ADDRESSES: &'static [Address];

    /// Statically validates the addresses exposed by this set.
    const CHECK_ADDRESSES: () = assert_valid_precompile_addresses(Self::ADDRESSES);

    /// Executes the precompile at `address`.
    ///
    /// Callers must check `Self::ADDRESSES` before dispatching. Implementations may assume
    /// `address` is one of the advertised addresses.
    fn execute<ST: TxState<S>>(
        &self,
        address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult;
}

/// A precompile set with no custom precompiles.
#[derive(Debug, Clone, Default)]
pub struct NoCustomPrecompiles<S>(PhantomData<S>);

impl<S: Spec> EvmPrecompileSet<S> for NoCustomPrecompiles<S> {
    const ADDRESSES: &'static [Address] = &[];

    fn execute<ST: TxState<S>>(
        &self,
        _address: Address,
        _input: &[u8],
        _gas_limit: u64,
        _env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        unreachable!("no custom precompiles are configured")
    }
}

#[doc(hidden)]
pub mod __private {
    pub use sov_modules_api::{Spec, TxState};
}

/// Generates an [`EvmPrecompileSet`] from a flat list of individual [`EvmPrecompile`]s.
///
/// The generated set owns one instance of each listed precompile and derives its static address
/// list from their `ADDRESS` constants.
#[macro_export]
macro_rules! generate_precompile_set {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident<$spec:ident> {
            $($field:ident: $precompile:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Default)]
        $vis struct $name<$spec: $crate::precompiles::__private::Spec> {
            $($field: $precompile,)*
        }

        impl<$spec> $crate::precompiles::EvmPrecompileSet<$spec> for $name<$spec>
        where
            $spec: $crate::precompiles::__private::Spec,
            $($precompile: $crate::precompiles::EvmPrecompile<$spec>,)*
        {
            const ADDRESSES: &'static [$crate::precompiles::Address] = &[
                $(<$precompile as $crate::precompiles::EvmPrecompile<$spec>>::ADDRESS,)*
            ];

            fn execute<ST: $crate::precompiles::__private::TxState<$spec>>(
                &self,
                address: $crate::precompiles::Address,
                input: &[u8],
                gas_limit: u64,
                env: &mut $crate::precompiles::EvmPrecompileEnv<'_, $spec, ST>,
            ) -> $crate::precompiles::PrecompileResult {
                $(
                    if address == <$precompile as $crate::precompiles::EvmPrecompile<$spec>>::ADDRESS {
                        return <$precompile as $crate::precompiles::EvmPrecompile<$spec>>::execute(
                            &self.$field,
                            input,
                            gas_limit,
                            env,
                        );
                    }
                )*

                unreachable!("provider pre-filters custom precompile addresses")
            }
        }
    };
}

const ETH_RESERVED_PRECOMPILE_ADDRESSES: &[Address] = &[
    eth_precompile_address(1),
    eth_precompile_address(2),
    eth_precompile_address(3),
    eth_precompile_address(4),
    eth_precompile_address(5),
    eth_precompile_address(6),
    eth_precompile_address(7),
    eth_precompile_address(8),
    eth_precompile_address(9),
    eth_precompile_address(0x0a),
    eth_precompile_address(0x0b),
    eth_precompile_address(0x0c),
    eth_precompile_address(0x0d),
    eth_precompile_address(0x0e),
    eth_precompile_address(0x0f),
    eth_precompile_address(0x10),
    eth_precompile_address(0x11),
    eth_precompile_address(0x100),
];

const fn assert_valid_precompile_addresses(addresses: &[Address]) {
    if !has_unique_addresses(addresses) {
        panic!("duplicate custom EVM precompile address");
    }
    if has_reserved_ethereum_precompile_collision(addresses) {
        panic!("custom EVM precompile address collides with Ethereum precompile");
    }
}

const fn has_unique_addresses(addresses: &[Address]) -> bool {
    let mut i = 0;
    while i < addresses.len() {
        let mut j = i + 1;
        while j < addresses.len() {
            if addresses[i].const_eq(&addresses[j]) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

const fn has_reserved_ethereum_precompile_collision(addresses: &[Address]) -> bool {
    let mut i = 0;
    while i < addresses.len() {
        if is_reserved_ethereum_precompile_address(&addresses[i]) {
            return true;
        }
        i += 1;
    }
    false
}

const fn is_reserved_ethereum_precompile_address(address: &Address) -> bool {
    let mut i = 0;
    while i < ETH_RESERVED_PRECOMPILE_ADDRESSES.len() {
        if address.const_eq(&ETH_RESERVED_PRECOMPILE_ADDRESSES[i]) {
            return true;
        }
        i += 1;
    }
    false
}

const fn eth_precompile_address(x: u64) -> Address {
    let x = x.to_be_bytes();
    Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::precompiles::{
        SequencingTimestampPrecompile, BANK_BALANCE_PRECOMPILE_ADDRESS,
        SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
    };

    #[test]
    fn address_validation_detects_duplicate_custom_addresses() {
        assert!(!has_unique_addresses(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS,
            BANK_BALANCE_PRECOMPILE_ADDRESS,
        ]));
    }

    #[test]
    fn address_validation_detects_ethereum_precompile_collisions() {
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(1)
        ]));
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(0x11)
        ]));
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(0x100)
        ]));
        assert!(!has_reserved_ethereum_precompile_collision(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS
        ]));
    }

    #[test]
    fn address_validation_accepts_built_in_custom_precompiles() {
        assert_valid_precompile_addresses(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS,
            SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
        ]);
        assert_eq!(
            <SequencingTimestampPrecompile<sov_test_utils::TestSpec> as EvmPrecompileSet<
                sov_test_utils::TestSpec,
            >>::ADDRESSES,
            &[SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS]
        );
    }
}
