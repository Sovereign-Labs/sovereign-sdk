#![no_main]

use std::collections::{HashMap, HashSet};

use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::{fuzz_target, Corpus};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngCore, SeedableRng};
use sov_accounts::{AccountConfig, Accounts, CallMessage, UPDATE_ACCOUNT_MSG};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, Module, PrivateKey, Spec, WorkingSet};

type C = DefaultContext;

// Check well-formed calls
fuzz_target!(|input: (u16, [u8; 32], Vec<DefaultPrivateKey>)| -> Corpus {
    let (iterations, seed, keys) = input;
    if iterations < 1024 {
        // pointless to setup & run a small iterations count
        return Corpus::Reject;
    }

    // this is a workaround to the restriction where `ed25519_dalek::Keypair` doesn't implement
    // `Eq` or `Sort`; reduce the set to a unique collection of keys so duplicated accounts are not
    // used.
    let keys = keys
        .into_iter()
        .map(|k| (k.as_hex(), k))
        .collect::<HashMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();

    if keys.is_empty() {
        return Corpus::Reject;
    }

    let rng = &mut StdRng::from_seed(seed);
    let mut seed = [0u8; 32];
    let tmpdir = tempfile::tempdir().unwrap();
    let storage = <C as Spec>::Storage::with_path(tmpdir.path()).unwrap();
    let working_set = &mut WorkingSet::new(storage);

    let config: AccountConfig<C> = keys.iter().map(|k| k.pub_key()).collect();
    let accounts: Accounts<C> = Accounts::default();
    accounts.genesis(&config, working_set).unwrap();

    // address list is constant for this test
    let mut used = keys.iter().map(|k| k.as_hex()).collect::<HashSet<_>>();
    let mut state: HashMap<_, _> = keys.into_iter().map(|k| (k.default_address(), k)).collect();
    let addresses: Vec<_> = state.keys().copied().collect();

    for _ in 0..iterations {
        // we use slices for better select performance
        let sender = addresses.choose(rng).unwrap();
        let context = C::new(*sender);

        // clear previous state
        let previous = state.get(sender).unwrap().as_hex();
        used.remove(&previous);

        // generate an unused key
        rng.fill_bytes(&mut seed);
        let u = &mut Unstructured::new(&seed);
        let mut secret = DefaultPrivateKey::arbitrary(u).unwrap();
        while used.contains(&secret.as_hex()) {
            rng.fill_bytes(&mut seed);
            let u = &mut Unstructured::new(&seed);
            secret = DefaultPrivateKey::arbitrary(u).unwrap();
        }
        used.insert(secret.as_hex());

        let public = secret.pub_key();
        let sig = secret.sign(&UPDATE_ACCOUNT_MSG);
        state.insert(*sender, secret);

        let msg = CallMessage::<C>::UpdatePublicKey(public.clone(), sig);
        accounts.call(msg, &context, working_set).unwrap();
    }

    Corpus::Keep
});
