use std::cell::RefCell;

use risc0_zkvm::receipt::Receipt;
use risc0_zkvm::serde::to_vec;
use risc0_zkvm::{
    Executor, ExecutorEnvBuilder, LocalExecutor, SegmentReceipt, Session, SessionReceipt,
};
use sov_rollup_interface::zk::{Zkvm, ZkvmHost};
use std::collections::HashMap;

use risc0_zkvm_platform::syscall::SyscallName;

use crate::Risc0MethodId;

pub struct Risc0Host<'a> {
    env: RefCell<ExecutorEnvBuilder<'a>>,
    elf: &'a [u8],
}


#[cfg(feature = "bench")]
use once_cell::sync::Lazy;
#[cfg(feature = "bench")]
use parking_lot::Mutex;

#[cfg(feature = "bench")]
pub static GLOBAL_HASHMAP: Lazy<Mutex<HashMap<String,(u64, u64)>>> = Lazy::new(|| {
    Mutex::new(HashMap::new())
});

#[cfg(feature = "bench")]
pub fn add_value(metric: String, value:  u64) {
    let mut hashmap = GLOBAL_HASHMAP.lock();
    hashmap.entry(metric)
        .and_modify(|(sum, count)| {
            *sum += value;
            *count += 1;
        })
        .or_insert((value, 1));
}

fn deserialize_custom(serialized: &[u8]) -> (String, u64) {
    let null_pos = serialized.iter().position(|&b| b == 0).unwrap();
    let (string_bytes, size_bytes_with_null) = serialized.split_at(null_pos);
    let size_bytes = &size_bytes_with_null[1..]; // Skip the null terminator
    let string = String::from_utf8(string_bytes.to_vec()).unwrap();
    let size = u64::from_ne_bytes(size_bytes.try_into().unwrap()); // Convert bytes back into usize
    let tuple = (string, size);
    tuple
}

impl<'a> Risc0Host<'a> {
    pub fn new(elf: &'a [u8]) -> Self {
        let mut default_env = ExecutorEnvBuilder::default();
        default_env.env_var("RISC0_EXPERIMENTAL_PREFLIGHT","1");

        let cycle_string = String::from("cycle_metrics\0");
        let metrics_syscall_name = unsafe {
            SyscallName::from_bytes_with_nul(cycle_string.as_ptr())
        };

        let metrics_callback = |input: &[u8]| -> Vec<u8> {
            #[cfg(feature = "bench")]
            {
                let met_tuple = deserialize_custom(input);
                add_value(met_tuple.0, met_tuple.1);
            }
            vec![]
        };

        default_env.io_callback(metrics_syscall_name, metrics_callback);

        Self {
            env: RefCell::new(default_env),
            elf,
        }
    }

    /// Run a computation in the zkvm without generating a receipt.
    /// This creates the "Session" trace without invoking the heavy cryptoraphic machinery.
    pub fn run_without_proving(&mut self) -> anyhow::Result<Session> {
        let env = self.env.borrow_mut().build()?;
        let mut executor = LocalExecutor::from_elf(env, self.elf)?;
        executor.run()
    }

    /// Run a computation in the zkvm and generate a receipt.
    pub fn run(&mut self) -> anyhow::Result<SessionReceipt> {
        let session = self.run_without_proving()?;
        session.prove()
    }

}

impl<'a> ZkvmHost for Risc0Host<'a> {
    fn write_to_guest<T: serde::Serialize>(&self, item: T) {
        let serialized = to_vec(&item).expect("Serialization to vec is infallible");
        self.env.borrow_mut().add_input(&serialized);
    }
}

impl<'prover> Zkvm for Risc0Host<'prover> {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        verify_from_slice(serialized_proof, code_commitment)
    }
}

pub struct Risc0Verifier;

impl Zkvm for Risc0Verifier {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        verify_from_slice(serialized_proof, code_commitment)
    }
}

fn verify_from_slice<'a>(
    serialized_proof: &'a [u8],
    code_commitment: &Risc0MethodId,
) -> Result<&'a [u8], anyhow::Error> {
    let Risc0Proof::<'a> {
        segment_receipts,
        journal,
        ..
    } = bincode::deserialize(serialized_proof)?;

    let receipts = segment_receipts
        .into_iter()
        .map(|r| r as Box<dyn Receipt>)
        .collect::<Vec<_>>();
    SessionReceipt::new(receipts, journal.to_vec()).verify(code_commitment.0)?;
    Ok(journal)
}

/// A convenience type which contains the same data a Risc0 [`SessionReceipt`] but borrows the journal
/// data. This allows to avoid one unnecessary copy during proof verification.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Risc0Proof<'a> {
    pub segment_receipts: Vec<Box<SegmentReceipt>>,
    pub journal: &'a [u8],
}
