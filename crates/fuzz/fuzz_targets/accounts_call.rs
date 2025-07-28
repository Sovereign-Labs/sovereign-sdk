#![no_main]

use std::collections::{HashMap, HashSet};

use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::{fuzz_target, Corpus};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngCore, SeedableRng};
use sov_accounts::{AccountConfig, AccountData, Accounts, CallMessage};
use sov_modules_api::capabilities::mocks::MockKernel;
use sov_modules_api::{
    Context, CredentialId, DaSpec, Module, PrivateKey, PublicKey, Spec, StateCheckpoint, WorkingSet,
};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::TestPrivateKey;

type S = sov_test_utils::TestSpec;
// Check well-formed calls
fuzz_target!(
    |input: (u16, [u8; 32], [u8; 28], [u8; 32], Vec<TestPrivateKey>)| -> Corpus {
        let (iterations, seed, sequencer, sequencer_da, keys) = input;
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
        let storage_manager = SimpleStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state = StateCheckpoint::<S>::new(storage, &MockKernel::<S>::default());

        let sequencer = <S as Spec>::Address::from(sequencer);
        let sequencer_da = <<S as Spec>::Da as DaSpec>::Address::from(sequencer_da);

        let accounts: Vec<_> = keys
            .iter()
            .map(|k| {
                let credential_id = k.pub_key().credential_id();

                AccountData {
                    credential_id,
                    address: credential_id.into(),
                }
            })
            .collect();

        let config = AccountConfig { accounts };

        let mut accounts: Accounts<S> = Accounts::default();
        let mut genesis_state = state.to_genesis_state_accessor::<Accounts<S>>(&config);
        accounts
            .genesis(&Default::default(), &config, &mut genesis_state)
            .unwrap();

        let mut working_set: WorkingSet<S> = state.to_working_set_unmetered();

        // address list is constant for this test
        let mut used = keys.iter().map(|k| k.as_hex()).collect::<HashSet<_>>();
        let mut state: HashMap<_, _> = keys
            .into_iter()
            .map(|k| {
                let credential_id = k.pub_key().credential_id();
                (credential_id.into(), k)
            })
            .collect();
        let addresses: Vec<_> = state.keys().copied().collect();

        for _ in 0..iterations {
            // we use slices for better select performance
            let sender = addresses.choose(rng).unwrap();
            let context = Context::<S>::new(*sender, Default::default(), sequencer, sequencer_da);

            // clear previous state
            let previous = state.get(sender).unwrap().as_hex();
            used.remove(&previous);

            // generate an unused key
            rng.fill_bytes(&mut seed);
            let u = &mut Unstructured::new(&seed);
            let mut secret = TestPrivateKey::arbitrary(u).unwrap();
            while used.contains(&secret.as_hex()) {
                rng.fill_bytes(&mut seed);
                let u = &mut Unstructured::new(&seed);
                secret = TestPrivateKey::arbitrary(u).unwrap();
            }
            used.insert(secret.as_hex());

            let public = secret.pub_key();
            state.insert(*sender, secret);

            let credential_id: CredentialId = public.credential_id();

            let msg = CallMessage::InsertCredentialId(credential_id);
            accounts.call(msg, &context, &mut working_set).unwrap();
        }

        Corpus::Keep
    }
);
