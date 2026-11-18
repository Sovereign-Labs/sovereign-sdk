use std::fmt;

/// Runtime configuration for [`crate::CelestiaFibreDaService`].
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CelestiaFibreConfig {
    /// Celestia bridge node RPC, for example `ws://localhost:26658`.
    /// Serves headers and the v2 shares that reference Fibre blobs.
    pub rpc_url: String,
    /// JWT token for the RPC server.
    pub rpc_auth_token: Option<String>,
    /// Celestia consensus node gRPC, for example `http://localhost:9090`.
    /// Fibre needs it for both downloads (validator set) and submissions.
    pub grpc_url: String,
    /// Optional gRPC authentication token, sent as `x-token` metadata.
    pub grpc_auth_token: Option<String>,
    /// Hex-encoded secp256k1 private key. Without it the service is read-only.
    pub signer_private_key: Option<String>,
}

impl fmt::Debug for CelestiaFibreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CelestiaFibreConfig")
            .field("rpc_url", &self.rpc_url)
            .field(
                "rpc_auth_token",
                &self.rpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("grpc_url", &self.grpc_url)
            .field(
                "grpc_auth_token",
                &self.grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field(
                "signer_private_key",
                &self.signer_private_key.as_ref().map(|_| "REDACTED"),
            )
            .finish()
    }
}
