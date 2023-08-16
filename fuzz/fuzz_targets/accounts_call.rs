#![no_main]

use libfuzzer_sys::fuzz_target;
use sov_accounts::{Accounts, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Module;
use sov_state::{ProverStorage, WorkingSet};

type C = DefaultContext;

fuzz_target!(|input: &[u8]| {
    if input.len() < 32 {
        return;
    }
    let (sender_bytes, data) = input.split_at(32);
    let sender: [u8; 32] = sender_bytes.try_into().unwrap();
    if let Ok(msgs) = serde_json::from_slice::<Vec<CallMessage<C>>>(data) {
        let tmpdir = tempfile::tempdir().unwrap();
        let ctx = C {
            sender: sender.into(),
        };
        let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

        let accounts = Accounts::default();
        for msg in msgs {
            accounts.call(msg, &ctx, &mut working_set).ok();
        }
    }
});
