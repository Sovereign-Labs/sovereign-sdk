use std::path::{Path, PathBuf};

use demo_stf::runtime::{Runtime, RuntimeCall, RuntimeSubcommand};
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::transactions::{ImportTransaction, TransactionWorkflow};
use sov_modules_api::cli::{FileStringArg, JsonStringArg};
use sov_modules_api::default_context::DefaultContext;

#[test]
fn test_import_transaction_from_string() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<RuntimeCall<DefaultContext>, DefaultContext>::default();

    let test_token_path = make_test_path("requests/create_token.json");
    let subcommand = RuntimeSubcommand::<JsonStringArg, DefaultContext>::bank {
        contents: JsonStringArg {
            json: std::fs::read_to_string(&test_token_path).unwrap(),
        },
    };

    let workflow = TransactionWorkflow::Import(ImportTransaction::<
        _,
        RuntimeSubcommand<JsonStringArg, DefaultContext>,
    >::FromFile(subcommand));
    workflow
        .run::<Runtime<DefaultContext>, _, _, _, _, _>(&mut wallet_state, app_dir)
        .unwrap();

    assert_eq!(wallet_state.unsent_transactions.len(), 1);
}

#[test]
fn test_import_transaction_from_file() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<RuntimeCall<DefaultContext>, DefaultContext>::default();

    let test_token_path = make_test_path("requests/create_token.json");
    let subcommand = RuntimeSubcommand::<FileStringArg, DefaultContext>::bank {
        contents: FileStringArg {
            path: test_token_path.to_str().unwrap().into(),
        },
    };

    let workflow = TransactionWorkflow::Import(ImportTransaction::<
        _,
        RuntimeSubcommand<JsonStringArg, DefaultContext>,
    >::FromFile(subcommand));
    workflow
        .run::<Runtime<DefaultContext>, _, _, _, _, _>(&mut wallet_state, app_dir)
        .unwrap();

    assert_eq!(wallet_state.unsent_transactions.len(), 1);
}

fn make_test_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let mut sender_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    sender_path.push("test-data");

    sender_path.push(path);

    sender_path
}
