pub mod breaker;
pub mod config;
pub mod credentials;
pub mod ewma;
pub mod http;
pub mod http_provider;
pub mod server;
pub mod state;
pub mod streaming;
pub mod types;

pub use breaker::{BreakerState, CircuitBreaker};
pub use config::provider_server_config_from_env;
pub use credentials::{read_keychain_secret, resolve_api_key, resolve_oauth_access_token};
pub use ewma::Ewma;
pub use http::{bearer_headers, http_client, join_url};
pub use http_provider::{HttpProvider, HttpProviderDescriptor, ProviderDialect};
pub use server::{ProviderServerConfig, serve_over_uds};
pub use state::{ProviderState, StateMap};
pub use streaming::{ProgressEvent, ProgressSink, parse_sse_lines};
pub use types::{
    ChatCompleteParams, ChatCompleteResult, ChatMessage, ModelInfo, ModelsListParams,
    ModelsListResult, ProbeParams, ProbeResult, Provider, ProviderCapability, ProviderError,
    RateLimit,
};
