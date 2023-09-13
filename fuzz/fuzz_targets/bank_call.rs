#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_bank::{Bank, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Module, WorkingSet};
use sov_state::ProverStorage;

type C = DefaultContext;

fuzz_target!(|input: (&[u8], [u8; 32])| {
    let (data, sender) = input;
    if let Ok(msgs) = serde_json::from_slice::<Vec<CallMessage<C>>>(data) {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
        let ctx = C {
            sender: sender.into(),
        };

        let bank = Bank::default();
        for msg in msgs {
            bank.call(msg, &ctx, &mut working_set).ok();
        }
    }
});
