// HOM Ingress — legacy bubble HTTP client.
// Kept for compatibility with older tests/callers; app-facing routes now use the capability mesh.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// Result of a bubble HTTP call.
pub enum BubbleResponse {
    /// Bubble responded with a status code and JSON body.
    Responded { status: StatusCode, body: Value },
    /// Bubble is unreachable — caller should return capability_unavailable.
    Offline {
        runtime: &'static str,
        method: &'static str,
        route: String,
    },
}

impl BubbleResponse {
    /// Convenience: bubble responded with 200 OK.
    pub fn ok(body: Value) -> Self {
        Self::Responded {
            status: StatusCode::OK,
            body,
        }
    }

    /// Convert to an axum Response.
    /// Offline responses return 503 with the capability_unavailable error shape.
    /// Successful responses forward the bubble's status code and body.
    pub fn into_response(self) -> Response {
        match self {
            BubbleResponse::Responded { status, body } => (status, Json(body)).into_response(),
            BubbleResponse::Offline {
                runtime,
                method,
                route,
            } => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": -32090,
                        "message": "capability_unavailable",
                        "data": {
                            "runtime": runtime,
                            "route": route,
                            "method": method,
                            "state": "offline",
                            "reason": format!("{runtime} offline"),
                            "required_action": format!("Install or start {runtime}."),
                            "brain_forwarded": false,
                            "retryable": true
                        }
                    }
                })),
            )
                .into_response(),
        }
    }
}

/// HTTP client for a single bubble runtime.
#[derive(Clone)]
pub struct BubbleClient {
    http: reqwest::Client,
    base_url: String,
    runtime: &'static str,
}

impl BubbleClient {
    /// Create a new bubble client.
    /// `base_url` should include the scheme and port.
    /// `runtime` is the display name for error messages, e.g. "ProviderRuntime".
    pub fn new(base_url: String, runtime: &'static str) -> Self {
        let timeout_ms = std::env::var("HOM_BUBBLE_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000);
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(timeout_ms))
                .build()
                .unwrap_or_default(),
            base_url,
            runtime,
        }
    }

    /// GET a path on the bubble runtime.
    pub async fn get(&self, path: &str) -> BubbleResponse {
        let url = format!("{}{}", self.base_url, path);
        match self.http.get(&url).send().await {
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                match resp.json::<Value>().await {
                    Ok(body) => BubbleResponse::Responded { status, body },
                    Err(_) => BubbleResponse::Responded {
                        status,
                        body: json!({"ok": false, "error": {"code": -32050, "message": "bubble_response_parse_error"}}),
                    },
                }
            }
            Err(_) => BubbleResponse::Offline {
                runtime: self.runtime,
                method: "GET",
                route: path.to_string(),
            },
        }
    }

    /// POST a JSON body to a path on the bubble runtime.
    pub async fn post(&self, path: &str, body: Value) -> BubbleResponse {
        let url = format!("{}{}", self.base_url, path);
        match self.http.post(&url).json(&body).send().await {
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                match resp.json::<Value>().await {
                    Ok(body) => BubbleResponse::Responded { status, body },
                    Err(_) => BubbleResponse::Responded {
                        status,
                        body: json!({"ok": false, "error": {"code": -32050, "message": "bubble_response_parse_error"}}),
                    },
                }
            }
            Err(_) => BubbleResponse::Offline {
                runtime: self.runtime,
                method: "POST",
                route: path.to_string(),
            },
        }
    }

    /// GET /health on the bubble runtime.
    pub async fn health(&self) -> BubbleResponse {
        self.get("/health").await
    }

    /// Return the runtime name.
    pub fn runtime(&self) -> &'static str {
        self.runtime
    }
}

/// Legacy collection of bubble clients. New ingress state does not route through these.
#[derive(Clone)]
pub struct BubbleClients {
    pub provider: BubbleClient,
    pub tools: BubbleClient,
    pub skills: BubbleClient,
    pub plugins: BubbleClient,
}

impl BubbleClients {
    /// Create all bubble clients from explicit URLs.
    pub fn from_urls(
        provider_url: String,
        tools_url: String,
        skills_url: String,
        plugins_url: String,
    ) -> Self {
        Self {
            provider: BubbleClient::new(provider_url, "ProviderRuntime"),
            tools: BubbleClient::new(tools_url, "ToolsRuntime"),
            skills: BubbleClient::new(skills_url, "SkillsRuntime"),
            plugins: BubbleClient::new(plugins_url, "PluginsRuntime"),
        }
    }

    /// Create all bubble clients from environment variables with disabled local defaults.
    pub fn from_env() -> Self {
        let provider_url = std::env::var("HOM_PROVIDER_RUNTIME_URL")
            .or_else(|_| std::env::var("CODEX_OAUTH_RUNTIME_URL"))
            .unwrap_or_else(|_| "http://127.0.0.1:9110".to_string());
        let tools_url = std::env::var("HOM_TOOLS_RUNTIME_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:1".to_string());
        let skills_url = std::env::var("HOM_SKILLS_RUNTIME_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:1".to_string());
        let plugins_url = std::env::var("HOM_PLUGINS_RUNTIME_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:1".to_string());

        Self::from_urls(provider_url, tools_url, skills_url, plugins_url)
    }

    /// Create all bubble clients with disabled URLs (for testing).
    pub fn new_defaults() -> Self {
        Self::from_urls(
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:1".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_client_defaults_do_not_use_sibling_runtime_ports() {
        let clients = BubbleClients::new_defaults();
        assert_eq!(clients.provider.base_url, "http://127.0.0.1:1");
        assert_eq!(clients.tools.base_url, "http://127.0.0.1:1");
        assert_eq!(clients.skills.base_url, "http://127.0.0.1:1");
        assert_eq!(clients.plugins.base_url, "http://127.0.0.1:1");
    }

    #[test]
    fn bubble_offline_response_shape() {
        let response = BubbleResponse::Offline {
            runtime: "ProviderRuntime",
            method: "GET",
            route: "/providers".to_string(),
        };
        let axum_response = response.into_response();
        assert_eq!(axum_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn bubble_responded_ok_response() {
        let response = BubbleResponse::ok(json!({"ok": true, "items": []}));
        let axum_response = response.into_response();
        assert_eq!(axum_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bubble_client_offline_when_unreachable() {
        // Point at a port that's definitely not running
        let client = BubbleClient::new("http://127.0.0.1:19999".to_string(), "TestRuntime");
        let result = client.get("/health").await;
        match result {
            BubbleResponse::Offline {
                runtime,
                method,
                route,
            } => {
                assert_eq!(runtime, "TestRuntime");
                assert_eq!(method, "GET");
                assert_eq!(route, "/health");
            }
            BubbleResponse::Responded { .. } => {
                panic!("Expected Offline when bubble is unreachable, got Responded");
            }
        }
    }
}
