use std::path::{Path, PathBuf};

use demo_stf::runtime::{JsonStringArg, Runtime, RuntimeCall, RuntimeSubcommand};
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::transactions::TransactionWorkflow;
use sov_modules_api::default_context::DefaultContext;

#[test]
fn test_import_transaction() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<RuntimeCall<DefaultContext>, DefaultContext>::default();

    let test_token_path = make_test_path("requests/create_token.json");
    let test_token_calldata = std::fs::read_to_string(test_token_path).unwrap();

    let workflow = TransactionWorkflow::Import(RuntimeSubcommand::<_, DefaultContext>::bank {
        contents: JsonStringArg {
            json: test_token_calldata,
        },
    });
    workflow
        .run::<Runtime<DefaultContext>, _, _, _, _>(&mut wallet_state, app_dir)
        .unwrap();

    assert_eq!(wallet_state.unsent_transactions.len(), 1);
}

fn make_test_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let mut sender_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    sender_path.push("test-data");

    sender_path.push(path);

    sender_path
}
