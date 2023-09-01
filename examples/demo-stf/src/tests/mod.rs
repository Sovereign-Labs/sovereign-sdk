use std::path::Path;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::mocks::MockDaSpec;
use sov_state::ProverStorage;

use crate::runtime::Runtime;

mod da_simulation;
mod stf_tests;
mod tx_revert_tests;
pub(crate) type C = DefaultContext;
pub(crate) type Da = MockDaSpec;

pub fn create_new_demo(
    path: impl AsRef<Path>,
) -> AppTemplate<
    DefaultContext,
    Da,
    sov_rollup_interface::mocks::MockZkvm,
    Runtime<DefaultContext, Da>,
> {
    let runtime = Runtime::default();
    let storage = ProverStorage::with_path(path).unwrap();
    AppTemplate::new(storage, runtime)
}
