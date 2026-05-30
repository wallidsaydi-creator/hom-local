use std::path::PathBuf;

use hom_brain::{App, ipc::server};
use hom_shared::{
    ensure_helper_identity, hom_local_dir, known_clients_path, load_known_clients, socket_path,
};

const INGRESS_SCOPES: &[&str] = &[
    "system",
    "memory:save",
    "memory:recall",
    "memory:open",
    "memory:answer",
    "events:read",
    "events:write",
    "diagnostics:read",
    "security:read",
    "gates:read",
    "gates:write",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let hom_dir = hom_local_dir();
    ensure_helper_identity(&hom_dir, "ingress", "ingress.local", INGRESS_SCOPES)?;

    let known_clients_file = std::env::var("HOM_KNOWN_CLIENTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| known_clients_path(&hom_dir));
    let known_clients = load_known_clients(&known_clients_file)?;
    let app = App::new(hom_dir.clone())?;

    let socket = std::env::var("HOM_BRAIN_SOCK")
        .or_else(|_| std::env::var("HOM_BRAIN_SOCKET"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| socket_path(hom_dir, "brain"));

    server::serve(socket, known_clients, app).await
}
