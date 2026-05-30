// SPDX-License-Identifier: Apache-2.0
use std::net::SocketAddr;
use std::sync::Arc;

use hom_ingress::brain_client::BrainClient;
use hom_ingress::http::{AppState, build_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let brain = BrainClient::from_env()?;
    let state = Arc::new(AppState::from_env(brain)?);
    let app = build_router(Arc::clone(&state));
    let bind = std::env::var("HOM_INGRESS_BIND").unwrap_or_else(|_| "127.0.0.1:9101".to_string());
    let addr: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    state.log_ready(&bind);
    axum::serve(listener, app).await?;
    Ok(())
}
