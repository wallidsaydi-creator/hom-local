use crate::server::ProviderServerConfig;
use std::io;

pub fn provider_server_config_from_env(helper_name: &str) -> io::Result<ProviderServerConfig> {
    Ok(ProviderServerConfig::from_env(helper_name))
}
