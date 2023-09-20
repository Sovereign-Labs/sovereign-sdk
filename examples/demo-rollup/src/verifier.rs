use demo_stf::runtime::Runtime;
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::DaVerifier;
use sov_stf_runner::verifier::StateTransitionVerifier;

/// A verifier for the demo rollup
pub type AppVerifier<DA, Zk> = StateTransitionVerifier<
    AppTemplate<
        ZkDefaultContext,
        <DA as DaVerifier>::Spec,
        Zk,
        Runtime<ZkDefaultContext, <DA as DaVerifier>::Spec>,
    >,
    DA,
    Zk,
>;
