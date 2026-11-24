//! Lets the stateless part of [`authenticate`](super::authenticate) run ahead of time, off the
//! thread that executes transactions.
//!
//! Between hashing the raw transaction and extracting its authorization data, `authenticate`
//! decodes the transaction, verifies its signature and computes its non-malleable hash. None of
//! this reads state: it is a function of the transaction bytes and of the chain hash the
//! transaction is verified against. [`prepare`] does that work once, against a [`RecordingMeter`],
//! and stores the results together with every gas charge the work made.
//!
//! `authenticate` then replays the recorded charges on the real meter, one by one, in the same
//! order and with the same arguments, instead of repeating the work. The meter therefore sees
//! exactly the calls it would have seen. If a replayed charge fails, the regular code path runs
//! against a [`ResumedMeter`], which makes it continue from that failed charge.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use sov_rollup_interface::TxHash;

use super::{
    authenticate_hashed_tx, decode_tx, finish_authentication, select_chain_hash, tx_details,
    verify_tx_multi_hash, AuthenticationError, AuthenticationOutput,
};
use crate::runtime::ResolvedChainHashes;
use crate::transaction::legacy_v0::DecodedTransaction;
use crate::transaction::Transaction;
use crate::{BasicGasMeter, DispatchCall, GasMeter, GasMeteringError, Spec};

/// The number of prepared transactions kept.
///
/// A replayed batch is prepared in full before its first transaction is executed, so this must be
/// larger than the number of transactions in a batch.
const PREPARED_TXS_CACHE_SIZE: usize = 131_072;

/// Prepared transactions by raw transaction hash. The values are [`PreparedTx`]s, type-erased
/// because a static cannot be generic over the runtime.
static PREPARED_TXS: LazyLock<quick_cache::sync::Cache<TxHash, Arc<dyn Any + Send + Sync>>> =
    LazyLock::new(|| quick_cache::sync::Cache::new(PREPARED_TXS_CACHE_SIZE));

/// One call made to a [`GasMeter`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedCharge<G> {
    /// A [`GasMeter::charge_gas`] call.
    Fixed(G),
    /// A [`GasMeter::charge_linear_gas`] call.
    Linear(G, u32),
}

impl<G: crate::Gas> RecordedCharge<G> {
    fn apply<M: GasMeter>(&self, meter: &mut M) -> Result<(), GasMeteringError<G>>
    where
        M::Spec: Spec<Gas = G>,
    {
        match self {
            RecordedCharge::Fixed(amount) => meter.charge_gas(*amount),
            RecordedCharge::Linear(amount, parameter) => {
                meter.charge_linear_gas(*amount, *parameter)
            }
        }
    }
}

/// A [`GasMeter`] that accepts every charge and records it.
struct RecordingMeter<S: Spec>(Vec<RecordedCharge<S::Gas>>);

impl<S: Spec> GasMeter for RecordingMeter<S> {
    type Spec = S;

    fn charge_gas(&mut self, amount: S::Gas) -> Result<(), GasMeteringError<S::Gas>> {
        self.0.push(RecordedCharge::Fixed(amount));
        Ok(())
    }

    fn charge_linear_gas(
        &mut self,
        amount: S::Gas,
        parameter: u32,
    ) -> Result<(), GasMeteringError<S::Gas>> {
        self.0.push(RecordedCharge::Linear(amount, parameter));
        Ok(())
    }
}

/// A [`GasMeter`] for re-running code whose first charges were already made on `inner`.
///
/// The charges in `already_charged` are skipped, because `inner` has seen them. The charge that
/// follows them was also made on `inner` and failed, so it returns that `failure` without touching
/// `inner` again. Everything after that goes to `inner`.
struct ResumedMeter<'a, M: GasMeter> {
    inner: &'a mut M,
    already_charged: std::slice::Iter<'a, RecordedCharge<<M::Spec as Spec>::Gas>>,
    failure: Option<GasMeteringError<<M::Spec as Spec>::Gas>>,
}

impl<M: GasMeter> ResumedMeter<'_, M> {
    /// Returns `None` if `charge` must be forwarded to `inner`.
    fn resume(
        &mut self,
        charge: &RecordedCharge<<M::Spec as Spec>::Gas>,
    ) -> Option<Result<(), GasMeteringError<<M::Spec as Spec>::Gas>>> {
        if let Some(expected) = self.already_charged.next() {
            debug_assert_eq!(
                charge, expected,
                "Re-running authentication must repeat the recorded gas charges"
            );
            return Some(Ok(()));
        }
        self.failure.take().map(Err)
    }
}

impl<M: GasMeter> GasMeter for ResumedMeter<'_, M> {
    type Spec = M::Spec;

    fn charge_gas(
        &mut self,
        amount: <M::Spec as Spec>::Gas,
    ) -> Result<(), GasMeteringError<<M::Spec as Spec>::Gas>> {
        let charge = RecordedCharge::Fixed(amount);
        match self.resume(&charge) {
            Some(result) => result,
            None => charge.apply(self.inner),
        }
    }

    fn charge_linear_gas(
        &mut self,
        amount: <M::Spec as Spec>::Gas,
        parameter: u32,
    ) -> Result<(), GasMeteringError<<M::Spec as Spec>::Gas>> {
        let charge = RecordedCharge::Linear(amount, parameter);
        match self.resume(&charge) {
            Some(result) => result,
            None => charge.apply(self.inner),
        }
    }

    fn try_as_basic_gas_meter(&mut self) -> Option<&mut BasicGasMeter<Self::Spec>> {
        self.inner.try_as_basic_gas_meter()
    }

    #[cfg(feature = "gas-constant-estimation")]
    fn remove_gas_pattern(&mut self, amount: &<Self::Spec as Spec>::Gas, parameter: u32) {
        self.inner.remove_gas_pattern(amount, parameter);
    }
}

/// A transaction that was decoded and verified ahead of [`authenticate`](super::authenticate).
pub(super) struct PreparedTx<D: DispatchCall> {
    tx: Transaction<D, D::Spec>,
    /// The chain hash the signature was verified against. The prepared results are only valid
    /// where `authenticate` selects the same one.
    chain_hash: [u8; 32],
    non_malleable_hash: TxHash,
    /// Every charge made while decoding and verifying `tx`, in order.
    charges: Vec<RecordedCharge<<D::Spec as Spec>::Gas>>,
}

/// Returns the prepared form of the transaction with hash `raw_tx_hash`, if there is one that is
/// valid for `resolved_hashes`.
pub(super) fn lookup<S: Spec, D: DispatchCall<Spec = S> + 'static>(
    raw_tx_hash: TxHash,
    resolved_hashes: &ResolvedChainHashes,
) -> Option<Arc<PreparedTx<D>>> {
    let prepared = PREPARED_TXS
        .get(&raw_tx_hash)?
        .downcast::<PreparedTx<D>>()
        .ok()?;
    // The chain hash decides the signing payload, and with it the signature check and the gas
    // charged for it. A chain hash mismatch is left to the regular path, which reports it.
    let selected =
        select_chain_hash(tx_details(&prepared.tx), resolved_hashes, raw_tx_hash).ok()?;
    (selected == prepared.chain_hash).then_some(prepared)
}

/// Decodes and verifies `raw_tx` and stores the result for [`lookup`].
///
/// Nothing is stored for a transaction that fails any of these steps or that uses the legacy V0
/// encoding: `authenticate` handles those on its regular path.
pub(super) fn prepare<S: Spec, D: DispatchCall<Spec = S> + 'static>(
    raw_tx: &[u8],
    raw_tx_hash: TxHash,
    resolved_hashes: &ResolvedChainHashes,
) {
    if lookup::<S, D>(raw_tx_hash, resolved_hashes).is_some() {
        return;
    }
    let mut meter = RecordingMeter::<S>(Vec::new());
    // A transaction that fails to decode is reported by `authenticate`.
    let Ok(DecodedTransaction::Current(tx)) = decode_tx::<S, D>(raw_tx, raw_tx_hash, &mut meter)
    else {
        return;
    };
    // A chain hash mismatch or a bad signature is reported by `authenticate`. The outcome of the
    // signature check is in the signature cache by now.
    let Ok((chain_hash, non_malleable_hash)) =
        verify_tx_multi_hash(raw_tx_hash, &tx, resolved_hashes, &mut meter)
    else {
        return;
    };
    let prepared: PreparedTx<D> = PreparedTx {
        tx,
        chain_hash,
        non_malleable_hash,
        charges: meter.0,
    };
    PREPARED_TXS.insert(raw_tx_hash, Arc::new(prepared));
}

impl<S: Spec, D: DispatchCall<Spec = S>> PreparedTx<D> {
    /// Does what [`authenticate_hashed_tx`] does for the transaction this was prepared from.
    pub(super) fn authenticate(
        &self,
        raw_tx: &[u8],
        raw_tx_hash: TxHash,
        height: u64,
        resolved_hashes: ResolvedChainHashes,
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<AuthenticationOutput<S, D::Decodable>, AuthenticationError> {
        for (index, charge) in self.charges.iter().enumerate() {
            if let Err(failure) = charge.apply(meter) {
                // The meter is where the regular path would be after failing the same charge.
                // Run the regular path from there, so that it decides what the failure means.
                let mut meter = ResumedMeter {
                    inner: meter,
                    already_charged: self.charges[..index].iter(),
                    failure: Some(failure),
                };
                return authenticate_hashed_tx::<S, D>(
                    raw_tx,
                    raw_tx_hash,
                    height,
                    resolved_hashes,
                    &mut meter,
                );
            }
        }
        finish_authentication(raw_tx_hash, &self.tx, self.non_malleable_hash, meter)
    }
}

#[cfg(test)]
mod tests {
    use sov_test_utils::MockZkvm;

    use super::*;
    use crate::default_spec::DefaultSpec;
    use crate::execution_mode::Native;
    use crate::{Gas, GasArray};

    type S = DefaultSpec<sov_mock_da::MockDaSpec, MockZkvm, MockZkvm, Native>;
    type TestGas = <S as Spec>::Gas;

    fn gas(first: u64, second: u64) -> TestGas {
        TestGas::from([first, second])
    }

    fn recorded() -> Vec<RecordedCharge<TestGas>> {
        vec![
            RecordedCharge::Fixed(gas(0x1234, 7)),
            RecordedCharge::Linear(gas(3, 0x0102), 0x0304),
            RecordedCharge::Fixed(gas(1, 0x4321)),
            RecordedCharge::Linear(gas(0x21, 2), 5),
        ]
    }

    /// Makes `charges` directly on a meter and returns what the meter was left with.
    fn charge_directly(
        charges: &[RecordedCharge<TestGas>],
        limit: TestGas,
    ) -> (Result<(), GasMeteringError<TestGas>>, TestGas) {
        let mut meter = BasicGasMeter::<S>::new_with_gas(limit, <TestGas as Gas>::Price::ZEROED);
        let result = charges
            .iter()
            .try_for_each(|charge| charge.apply(&mut meter));
        (result, meter.gas_info().gas_used)
    }

    /// Makes the first `resumed_at` charges and one failing charge on a meter, then runs all of
    /// `charges` through a [`ResumedMeter`], as the fallback of [`PreparedTx::authenticate`] does.
    fn charge_resumed(
        charges: &[RecordedCharge<TestGas>],
        limit: TestGas,
        resumed_at: usize,
    ) -> (Result<(), GasMeteringError<TestGas>>, TestGas) {
        let mut meter = BasicGasMeter::<S>::new_with_gas(limit, <TestGas as Gas>::Price::ZEROED);
        for charge in &charges[..resumed_at] {
            charge.apply(&mut meter).unwrap();
        }
        let failure = charges[resumed_at].apply(&mut meter).unwrap_err();
        let mut resumed = ResumedMeter {
            inner: &mut meter,
            already_charged: charges[..resumed_at].iter(),
            failure: Some(failure),
        };
        let result = charges
            .iter()
            .try_for_each(|charge| charge.apply(&mut resumed));
        (result, meter.gas_info().gas_used)
    }

    #[test]
    fn recording_meter_records_charges_in_order() {
        let mut meter = RecordingMeter::<S>(Vec::new());
        for charge in recorded() {
            charge.apply(&mut meter).unwrap();
        }

        assert_eq!(meter.0, recorded());
    }

    #[test]
    fn resumed_meter_fails_like_direct_charges() {
        let charges = recorded();

        for resumed_at in 0..charges.len() {
            // One unit short, in each dimension, of what the charge at `resumed_at` needs.
            let (_, used) = charge_directly(&charges[..=resumed_at], TestGas::MAX);
            let limit = used.checked_sub(gas(1, 1)).unwrap();

            assert_eq!(
                charge_resumed(&charges, limit, resumed_at),
                charge_directly(&charges, limit),
                "Resuming at charge {resumed_at} must leave the meter as direct charges do"
            );
        }
    }
}
