use sov_db::ledger_db::LedgerDb;
use sov_ledger_apis::{LedgerRoutes, LedgerState};
use sov_modules_api::capabilities::HasCapabilities;
use sov_modules_api::execution_mode::ExecutionMode;
use sov_modules_api::prelude::utoipa_swagger_ui::Config;
use sov_modules_api::rest::utils::errors;
use sov_modules_api::rest::{HasRestApi, StateUpdateReceiver};
use sov_modules_api::{
    BatchSequencerReceipt, NodeEndpoints, RuntimeEventProcessor, Spec, SyncStatus, *,
};
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_rollup_apis::{DefaultRollupStateProvider, RollupTxRouter};
use sov_stf_runner::{RollupConfig, RunnerConfig};

use super::SequencerCreationReceipt;
use crate::FullNodeBlueprint;

/// Register rollup's default RPC methods and Axum router.
#[allow(clippy::too_many_arguments)]
pub async fn register_endpoints<B, M>(
    state_update_receiver: StateUpdateReceiver<<B::Spec as Spec>::Storage>,
    sync_status_receiver: tokio::sync::watch::Receiver<SyncStatus>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ledger_db: &LedgerDb,
    sequencer: &SequencerCreationReceipt<B::Spec>,
    config: &RollupConfig<<B::Spec as Spec>::Address, B::DaService>,
) -> anyhow::Result<NodeEndpoints>
where
    B: FullNodeBlueprint<M> + 'static,
    M: ExecutionMode + 'static,
    B::Runtime: RuntimeEventProcessor + HasRestApi<B::Spec> + HasCapabilities<B::Spec>,
{
    let mut endpoints = B::Runtime::endpoints(sequencer.api_state.clone());

    // Sequencer endpoints.
    endpoints.axum_router = endpoints
        .axum_router
        .merge(sequencer.endpoints.axum_router.clone());
    endpoints
        .jsonrpsee_module
        .merge(sequencer.endpoints.jsonrpsee_module.clone())
        .map_err(|e| anyhow::anyhow!("Failed to merge sequencer JSON-RPC module: {e}"))?;

    // Ledger endpoint.
    {
        let ledger_axum_router =
            LedgerRoutes::<
                LedgerDb,
                // Can keep hard-coding:
                // BatchSequencerReceipt<B::DaSpec>,
                // or use some associated type.
                // TODO: But ideally it needs to be addressed properly: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1268
                BatchSequencerReceipt<B::Spec>,
                TxReceiptContents<B::Spec>,
                <B::Runtime as RuntimeEventProcessor>::RuntimeEvent,
            >::axum_router(ledger_db.clone(), shutdown_receiver.clone());
        let ledger_state = LedgerState {
            ledger: ledger_db.clone(),
            shutdown_receiver,
        };
        endpoints.axum_router = endpoints
            .axum_router
            .merge(ledger_axum_router.with_state(ledger_state));
    }

    // Rollup endpoint
    {
        let rollup_router = RollupTxRouter::<
            std::sync::Arc<DefaultRollupStateProvider<B::Spec, B::Runtime>>,
        >::axum_router(
            state_update_receiver,
            sequencer.da_address.clone(),
            config.sequencer.rollup_address.clone(),
            sync_status_receiver,
        );
        endpoints.axum_router = endpoints.axum_router.merge(rollup_router);
    }

    endpoints.axum_router = endpoints.axum_router.route(
        "/healthcheck",
        sov_modules_api::prelude::axum::routing::get(|| async { "ok" }),
    );
    endpoints.axum_router = endpoints.axum_router.fallback(errors::global_404);

    // Even if runtime does not have Open API spec, we still want to plug in Sequencer and Ledger.
    let mut runtime_spec = B::Runtime::default().openapi_spec().unwrap_or_default();
    runtime_spec.info.title = "Sovereign SDK Rollup JSON API".to_string();
    runtime_spec.info.description =
        Some("Sovereign SDK Runtime, Ledger and Sequencer JSON API".to_string());

    // Specs
    let serialized_runtime = sov_modules_api::prelude::serde_yaml::to_string(&runtime_spec)?;
    let mut combined_spec: openapiv3::OpenAPI =
        sov_modules_api::prelude::serde_yaml::from_str(&serialized_runtime)?;

    merge_specs(&mut combined_spec, sov_api_spec::open_api_v3_spec(), "")?;

    combined_spec.servers = vec![server_url_from_runner_config(&config.runner)];

    endpoints.axum_router = endpoints.axum_router.merge(
        sov_modules_api::prelude::utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
            .external_url_unchecked("/openapi-v3.json", serde_json::to_value(&combined_spec)?)
            .config(Config::from("/openapi-v3.json")),
    );

    Ok(endpoints)
}

fn merge_specs(
    into: &mut openapiv3::OpenAPI,
    from: openapiv3::OpenAPI,
    sub_path: &str,
) -> anyhow::Result<()> {
    let openapiv3::OpenAPI {
        paths: from_paths,
        components: from_components,
        tags,
        // Skipped intentionally, and shown explicitly which are skipped.
        // If new fields are added not having ".." will break this function,
        // bringing out attention to it.
        openapi: _,
        info: _,
        servers: _,
        extensions: _,
        security: _,
        external_docs: _,
    } = from;

    for (path, item) in from_paths.into_iter() {
        into.paths.paths.insert(format!("{sub_path}{path}"), item);
    }

    into.tags.extend_from_slice(&tags);

    if let Some(from_components) = from_components {
        let openapiv3::Components {
            schemas,
            responses,
            parameters,
            examples,
            request_bodies,
            headers,
            security_schemes,
            links,
            callbacks,
            // Skipped on purpose: causes issues with utoipa anyway.
            extensions: _extensions,
        } = from_components;
        let runtime_components = into.components.get_or_insert(Default::default());
        for (name, schema) in schemas {
            if runtime_components
                .schemas
                .insert(name.clone(), schema)
                .is_some()
            {
                anyhow::bail!("multiple schemas with name {}", name);
            };
        }

        for (name, response) in responses {
            if runtime_components
                .responses
                .insert(name.clone(), response)
                .is_some()
            {
                anyhow::bail!("multiple responses with name {}", name);
            }
        }

        for (name, parameter) in parameters {
            if runtime_components
                .parameters
                .insert(name.clone(), parameter)
                .is_some()
            {
                anyhow::bail!("multiple parameters with name {}", name);
            };
        }

        for (name, example) in examples {
            runtime_components.examples.insert(name, example);
        }

        for (name, request_body) in request_bodies {
            runtime_components.request_bodies.insert(name, request_body);
        }

        for (name, header) in headers {
            runtime_components.headers.insert(name, header);
        }

        for (name, security_scheme) in security_schemes {
            runtime_components
                .security_schemes
                .insert(name, security_scheme);
        }

        for (name, link) in links {
            runtime_components.links.insert(name, link);
        }

        for (name, callback) in callbacks {
            runtime_components.callbacks.insert(name, callback);
        }
    }
    Ok(())
}

fn server_url_from_runner_config(runner_config: &RunnerConfig) -> openapiv3::Server {
    let server_url = match &runner_config.http_config.public_address {
        None => format!(
            "http://{}:{}",
            runner_config.http_config.bind_host, runner_config.http_config.bind_port
        ),
        Some(public_url) => public_url
            .strip_suffix('/')
            .unwrap_or(public_url)
            .to_string(),
    };

    openapiv3::Server {
        url: server_url,
        description: Some("Default".to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use openapiv3::{
        IntegerType, OpenAPI, Operation, PathItem, ReferenceOr, Schema, SchemaKind, Type,
    };

    use super::*;

    #[test]
    fn test_merge_empty_specs() {
        let mut spec1 = OpenAPI::default();
        let spec2 = OpenAPI::default();

        merge_specs(&mut spec1, spec2, "/spec2").unwrap();
    }

    #[test]
    fn test_merge_happy_path() {
        let mut spec1 = OpenAPI::default();
        insert_schemas(&mut spec1, &["Schema1", "Schema2"]);
        insert_paths(&mut spec1, &["/Path1", "/Path2"]);
        insert_responses(&mut spec1, &["Response1", "Response2"]);

        let mut spec2 = OpenAPI::default();
        insert_schemas(&mut spec2, &["Schema3", "Schema4"]);
        // Same paths are fine, since it is going to be merged under
        insert_paths(&mut spec2, &["/Path1", "/Path2"]);
        insert_responses(&mut spec1, &["Response3", "Response4"]);

        merge_specs(&mut spec1, spec2, "/spec2").unwrap();

        let components = spec1.components.as_ref().expect("Components should exist");
        assert!(components.schemas.contains_key("Schema1"));
        assert!(components.schemas.contains_key("Schema2"));
        assert!(components.schemas.contains_key("Schema3"));
        assert!(components.schemas.contains_key("Schema4"));

        assert!(components.responses.contains_key("Response1"));
        assert!(components.responses.contains_key("Response2"));
        assert!(components.responses.contains_key("Response3"));
        assert!(components.responses.contains_key("Response4"));

        assert!(spec1.paths.paths.contains_key("/Path1"));
        assert!(spec1.paths.paths.contains_key("/Path2"));
        assert!(spec1.paths.paths.contains_key("/spec2/Path1"));
        assert!(spec1.paths.paths.contains_key("/spec2/Path2"));
    }

    // TODO: Same sub_path with "". (Add error?)

    #[test]
    fn test_merge_same_components() {
        let mut spec1 = OpenAPI::default();
        insert_schemas(&mut spec1, &["Schema1"]);

        let mut spec2 = OpenAPI::default();
        insert_schemas(&mut spec2, &["Schema1"]);

        let err = merge_specs(&mut spec1, spec2, "/spec2").unwrap_err();

        assert_eq!("multiple schemas with name Schema1", err.to_string());
    }

    #[test]
    fn test_merge_same_responses() {
        let mut spec1 = OpenAPI::default();
        insert_responses(&mut spec1, &["Response1"]);

        let mut spec2 = OpenAPI::default();
        insert_responses(&mut spec2, &["Response1"]);

        let err = merge_specs(&mut spec1, spec2, "/spec2").unwrap_err();

        assert_eq!("multiple responses with name Response1", err.to_string());
    }

    fn insert_schemas(spec: &mut OpenAPI, names: &[&str]) {
        let schema = Schema {
            schema_data: Default::default(),
            schema_kind: SchemaKind::Type(Type::Integer(IntegerType::default())),
        };

        let components = spec.components.get_or_insert(Default::default());

        for &name in names {
            components
                .schemas
                .insert(name.to_string(), ReferenceOr::Item(schema.clone()));
        }
    }

    fn insert_paths(spec: &mut OpenAPI, paths: &[&str]) {
        let path_item = PathItem {
            get: Some(Operation::default()),
            ..Default::default()
        };

        for &path in paths {
            spec.paths
                .paths
                .insert(path.to_string(), ReferenceOr::Item(path_item.clone()));
        }
    }

    fn insert_responses(spec: &mut OpenAPI, responses: &[&str]) {
        let components = spec.components.get_or_insert(Default::default());
        for &response in responses {
            components
                .responses
                .insert(response.to_string(), ReferenceOr::Item(Default::default()));
        }
    }

    fn runner_config(
        bind_host: &str,
        bind_port: u16,
        public_address: Option<&str>,
    ) -> RunnerConfig {
        RunnerConfig {
            genesis_height: 0,
            da_polling_interval_ms: 0,
            http_config: sov_stf_runner::HttpServerConfig {
                bind_host: bind_host.to_string(),
                bind_port,
                public_address: public_address.map(|s| s.to_string()),
                cors: sov_stf_runner::CorsConfiguration::Permissive,
            },
            save_tx_bodies: false,
            concurrent_sync_tasks: None,
        }
    }

    #[test]
    fn test_server_from_runner_config() {
        let config_1 = runner_config("192.168.0.201", 35786, None);
        let server_1 = server_url_from_runner_config(&config_1);
        assert_eq!("http://192.168.0.201:35786", server_1.url);

        let config_2 = runner_config("192.168.0.202", 35786, Some("https://rollup.sovereign.xyz"));
        let server_2 = server_url_from_runner_config(&config_2);
        assert_eq!("https://rollup.sovereign.xyz", server_2.url);

        let config_3 = runner_config(
            "192.168.0.202",
            35786,
            Some("https://rollup.sovereign.xyz/"),
        );
        let server_3 = server_url_from_runner_config(&config_3);
        assert_eq!("https://rollup.sovereign.xyz", server_3.url);
    }
}
