#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_bank::{Bank, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Context, Module, WorkingSet};
use sov_prover_storage_manager::new_orphan_storage;

type C = DefaultContext;

fuzz_target!(|input: (&[u8], [u8; 32], [u8; 32])| {
    let (data, sender, sequencer) = input;
    if let Ok(msgs) = serde_json::from_slice::<Vec<CallMessage<C>>>(data) {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut working_set = WorkingSet::new(new_orphan_storage(tmpdir.path()).unwrap());
        let ctx = C::new(sender.into(), sequencer.into(), 1);
        let bank = Bank::default();
        for msg in msgs {
            bank.call(msg, &ctx, &mut working_set).ok();
        }
    }
});
