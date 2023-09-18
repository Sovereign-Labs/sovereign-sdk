#![no_main]

use libfuzzer_sys::arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use sov_accounts::{Accounts, CallMessage};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Module, Spec, WorkingSet};

type C = DefaultContext;

// Check arbitrary, random calls
fuzz_target!(|input: (&[u8], Vec<(C, CallMessage<C>)>)| {
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = <C as Spec>::Storage::with_path(tmpdir.path()).unwrap();
    let working_set = &mut WorkingSet::new(storage);

    let (seed, msgs) = input;
    let u = &mut Unstructured::new(seed);
    let accounts: Accounts<C> = Accounts::arbitrary_workset(u, working_set).unwrap();

    for (ctx, msg) in msgs {
        // assert malformed calls won't panic
        accounts.call(msg, &ctx, working_set).ok();
    }
});
