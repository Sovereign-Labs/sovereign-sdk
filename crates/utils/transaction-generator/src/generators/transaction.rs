use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sov_mock_da::MockDaSpec;
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::{
    Amount, CryptoSpec, DispatchCall, Gas, GasArray, PrivateKey as _, Runtime, Spec, TxEffect,
};
use sov_modules_stf_blueprint::get_gas_used;
use sov_state::{DefaultStorageSpec, ProverStorage};
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    TransactionAssertContext, TransactionTestCase, TransactionType, TEST_DEFAULT_MAX_FEE,
    TEST_DEFAULT_MAX_PRIORITY_FEE,
};

use super::basic::{BasicChangeLogEntry, BasicModuleRef, BasicTag};
use crate::{Distribution, GeneratedMessage, MessageValidity, State};

/// Prepare the testing environment immediately before execution.
pub trait PrepareEnv<S> {
    /// Context that is used for setup.
    type Input;

    /// Prepare the testing environment.
    fn prepare_env(&mut self, input: &mut Self::Input);
}

/// Assert the outcome of applying some state update.
pub trait AssertOutcome<S> {
    /// The result of the state update.
    type Output;

    /// Asserts the outcome based on the provided output.
    fn assert_outcome(&self, output: &Self::Output);
}

/// A trait representing a generated transaction.
pub trait GeneratedTransaction
where
    Self: Sized,
{
    /// The type of the transaction.
    type Transaction;

    /// Context used to create the generated transaction.
    type Context<'a>;

    /// Constructor used to create the instance.
    fn new(context: &mut Self::Context<'_>) -> Self;

    /// The concrete transaction that was generated.
    fn transaction(&self) -> Self::Transaction;
}

/// Executes the implementor as a test case.
pub trait RunTest<S: Spec, RT: Runtime<S>> {
    /// Consumes self and executes a test case using self as input.
    fn run_test(self, runner: &mut TestRunner<RT, S>);
}

/// A transaction outcome associated with the `max_fee` field.
#[derive(Debug)]
pub enum MaxFeeOutcome {
    /// The provided max_fee was below the gas consumed. This should result in a failed transaction.
    Insufficient,
    /// The provided max_fee was exactly the same as the gas consumed. This should result in a
    /// successful transaction.
    Exact,
    /// The provided max_fee exceeded the gas consumed. This should result in a successful
    /// transaction.
    Excess,
}

impl<S: Spec> AssertOutcome<S> for MaxFeeOutcome {
    type Output = TxEffect<S>;

    fn assert_outcome(&self, output: &Self::Output) {
        match self {
            MaxFeeOutcome::Insufficient => assert!(
                !output.is_successful(),
                "Insufficient expected tx to not be successful, found {output:?}"
            ),
            MaxFeeOutcome::Exact => assert!(
                output.is_successful(),
                "Exact expected successful receipt, found {output:?}"
            ),
            MaxFeeOutcome::Excess => assert!(
                output.is_successful(),
                "Excess expected successful receipt, found {output:?}"
            ),
        }
    }
}

impl MaxFeeOutcome {
    const DIFF: Amount = Amount::new(2000);
    fn set_max_fee<S: Spec>(&self, gas_used: Amount, details: &mut TxDetails<S>) {
        details.max_fee = match self {
            MaxFeeOutcome::Insufficient => gas_used
                .checked_sub(Self::DIFF)
                .expect("Insufficient gas used"),
            MaxFeeOutcome::Exact => gas_used,
            MaxFeeOutcome::Excess => gas_used.checked_add(Self::DIFF).expect("Excess gas used"),
        };
    }
}

/// A transaction outcome associated with the `gas_limit` field.
#[derive(Debug)]
pub enum GasLimitOutcome {
    /// The transaction will fail due to insufficient gas caused by the gas_limit setting.
    Insufficient,
    /// The transaction will succeed due to sufficient gas caused by the gas_limit setting.
    Excess,
}

impl GasLimitOutcome {
    fn set_gas_limit<S: Spec>(&self, mut gas_used: S::Gas, details: &mut TxDetails<S>) {
        let gas_limit = match self {
            GasLimitOutcome::Insufficient => gas_used.scalar_sub(2000),
            GasLimitOutcome::Excess => gas_used.scalar_add(2000),
        };
        details.gas_limit = Some(gas_limit.clone());
    }
}

impl<S: Spec> AssertOutcome<S> for GasLimitOutcome {
    type Output = TxEffect<S>;

    fn assert_outcome(&self, output: &Self::Output) {
        match self {
            GasLimitOutcome::Insufficient => assert!(
                !output.is_successful(),
                "Insufficient expected tx to not be successful, found {output:?}"
            ),
            GasLimitOutcome::Excess => assert!(
                output.is_successful(),
                "Excess expected successful receipt, found {output:?}"
            ),
        }
    }
}

/// Outcomes associated with fields on transactions.
#[derive(Debug)]
pub enum TransactionOutcome {
    /// An outcome associated with the max_fee field.
    MaxFee(MaxFeeOutcome),
    /// An outcome associated with the gas_limit field.
    GasLimit(GasLimitOutcome),
}

type DefaultSpecWithHasher<S> = DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>;

/// Generated transaction implementation using the standard Sovereign SDK transaction struct.
#[derive(Clone)]
pub struct SovereignGeneratedTransaction<
    S: Spec<Storage = ProverStorage<DefaultSpecWithHasher<S>>, Da = MockDaSpec>,
    RT: Runtime<S> + MinimalGenesis<S> + DispatchCall,
> {
    /// The generated transaction to be executed by the test runner.
    pub tx: TransactionType<RT, S>,
    /// The outcome that applying the transaction should produce.
    pub outcome: Arc<TransactionOutcome>,
    /// The call message that was generated and used in [`Self::tx`].
    pub msg: GeneratedMessage<S, <RT as DispatchCall>::Decodable, BasicChangeLogEntry<S>>,
}

impl<S, RT> PrepareEnv<S> for SovereignGeneratedTransaction<S, RT>
where
    RT: Runtime<S> + MinimalGenesis<S> + DispatchCall,
    S: Spec<Storage = ProverStorage<DefaultSpecWithHasher<S>>, Da = MockDaSpec>,
{
    type Input = TestRunner<RT, S>;

    fn prepare_env(&mut self, input: &mut Self::Input) {
        let (simulated, _, _) = input.simulate(self.tx.clone());
        let batch_receipt = simulated.batch_receipts[0].clone();
        let tx_receipt = &simulated.batch_receipts[0].tx_receipts[0].clone();
        let gas_used = get_gas_used(tx_receipt);
        let gas_price = batch_receipt.inner.gas_price.clone();
        let gas_used_value = gas_used.value(&gas_price);
        let tx_details = self.tx.details_mut().unwrap();

        match &*self.outcome {
            TransactionOutcome::MaxFee(max_fee_outcome) => {
                max_fee_outcome.set_max_fee(gas_used_value, tx_details);
            }
            TransactionOutcome::GasLimit(gas_limit_outcome) => {
                gas_limit_outcome.set_gas_limit(gas_used, tx_details);
            }
        }
    }
}

impl<S, RT> AssertOutcome<S> for SovereignGeneratedTransaction<S, RT>
where
    S: Spec<Storage = ProverStorage<DefaultSpecWithHasher<S>>, Da = MockDaSpec>,
    RT: Runtime<S> + MinimalGenesis<S> + DispatchCall,
{
    type Output = TransactionAssertContext<S, RT>;

    fn assert_outcome(&self, output: &Self::Output) {
        match &*self.outcome {
            TransactionOutcome::MaxFee(max_fee_outcome) => {
                max_fee_outcome.assert_outcome(&output.tx_receipt);
            }
            TransactionOutcome::GasLimit(gas_limit_outcome) => {
                gas_limit_outcome.assert_outcome(&output.tx_receipt);
            }
        };
    }
}

/// Context used to create a [`SovereignGeneratedTransaction`] instance.
pub struct SovereignContext<'a, S: Spec, RT: DispatchCall + Runtime<S>> {
    /// A distribution of modules used to produce the call message.
    pub modules: Distribution<BasicModuleRef<S, RT>>,
    /// Used to create arbitrary instances
    pub u: &'a mut arbitrary::Unstructured<'a>,
    /// Generator state used for call message generation.
    pub call_generator_state: &'a mut State<S, BasicTag, ()>,
    /// A distribution of outcomes that influences the outcome of executing the transaction.
    pub outcomes: Distribution<Arc<TransactionOutcome>>,
}

impl<S, RT> GeneratedTransaction for SovereignGeneratedTransaction<S, RT>
where
    RT: Runtime<S> + MinimalGenesis<S> + DispatchCall,
    S: Spec<Storage = ProverStorage<DefaultSpecWithHasher<S>>, Da = MockDaSpec>,
{
    type Transaction = TransactionType<RT, S>;

    type Context<'a> = SovereignContext<'a, S, RT>;

    fn new(context: &mut Self::Context<'_>) -> Self {
        let module = context.modules.select_value(context.u).unwrap();
        let msg = module
            .generate_call_message(
                context.u,
                context.call_generator_state,
                // we always want these messages to be valid
                // we want the outcome to be based on transaction level attributes
                // i.e gas, nonces, etc.
                MessageValidity::Valid,
            )
            .unwrap();
        let outcome = context.outcomes.select_value(context.u).unwrap();
        let tx = TransactionType::<RT, S>::Plain {
            message: msg.message.clone(),
            key: msg.sender.clone(),
            details: TxDetails {
                max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
                max_fee: TEST_DEFAULT_MAX_FEE,
                gas_limit: None,
                chain_id: config_chain_id(),
            },
        };

        Self {
            tx,
            outcome: outcome.clone(),
            msg,
        }
    }

    fn transaction(&self) -> Self::Transaction {
        self.tx.clone()
    }
}

impl<S, RT> RunTest<S, RT> for SovereignGeneratedTransaction<S, RT>
where
    RT: Runtime<S> + MinimalGenesis<S> + DispatchCall,
    S: Spec<Storage = ProverStorage<DefaultSpecWithHasher<S>>, Da = MockDaSpec>,
{
    fn run_test(mut self, runner: &mut TestRunner<RT, S>) {
        self.prepare_env(runner);

        let input = self.transaction();
        let pubkey_for_nonce_to_decrement = match &input {
            TransactionType::Plain { key, .. } => Some(key.pub_key()),
            _ => None,
        };
        // we don't know ahead of time if the transaction is going to be skipped or reverted
        // so we always run `execute_transaction` instead of `execute_skipped_transaction`
        // and manually decrement the nonce ourselves if the tx was skipped
        let should_decrement_nonce = Arc::new(AtomicBool::new(false));
        let nonce_flag = should_decrement_nonce.clone();

        runner.execute_transaction(TransactionTestCase {
            input,
            assert: Box::new(move |context, _state| {
                self.assert_outcome(&context);

                if context.tx_receipt.is_skipped() {
                    nonce_flag.store(true, Ordering::Release);
                }
            }),
        });

        if should_decrement_nonce.load(Ordering::Acquire) {
            if let Some(pk) = pubkey_for_nonce_to_decrement {
                if let Some(n) = runner.nonces_mut().get_mut(&pk) {
                    *n -= 1;
                }
            }
        }
    }
}
