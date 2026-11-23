//! Module config: FIBRE protocol options + UDP bind.

use crate::wire::FibreConfig;
use blvm_sdk_macros::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::str::FromStr;

/// Static FIBRE peer for outbound block sends (`NewBlock` → `send_block`).
///
/// Declare in `config.toml` as `[[fibre_peers]]` with `peer_id` and `udp_addr`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FibrePeerEntry {
    /// Logical peer name (used when sending; matches in-process FIBRE peer id).
    pub peer_id: String,
    /// UDP destination `host:port` for FEC chunks.
    pub udp_addr: String,
}

/// Parsed config for [`crate::FibreModule`].
///
/// `udp_bind` unset listens on the node P2P address with the port plus one.
/// A set value is that socket. An empty value is an error.
#[config(name = "blvm-fibre")]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FibreModuleConfig {
    /// FIBRE protocol options (FEC, timeouts, etc.).
    #[serde(default)]
    pub fibre: FibreConfig,
    /// UDP listen address (`host:port`). Unset follows the node P2P port plus one.
    #[serde(default)]
    #[config_env]
    pub udp_bind: Option<String>,
    /// Outbound FIBRE targets (optional until dynamic registration exists).
    #[serde(default)]
    pub fibre_peers: Vec<FibrePeerEntry>,
    /// Register FIBRE UDP targets when a P2P peer advertises NODE_FIBRE (UDP = their P2P port + 1).
    #[serde(default = "default_register_peers_from_p2p")]
    #[config_env]
    pub register_peers_from_p2p: bool,
}

fn default_register_peers_from_p2p() -> bool {
    true
}

impl Default for FibreModuleConfig {
    fn default() -> Self {
        Self {
            fibre: FibreConfig::default(),
            udp_bind: None,
            fibre_peers: Vec::new(),
            register_peers_from_p2p: default_register_peers_from_p2p(),
        }
    }
}

blvm_sdk::impl_module_config!(FibreModuleConfig);

impl FibreModuleConfig {
    /// Fail when a file or node override still names the old bind switch or the enable flag.
    pub fn reject_retired(path: &Path) -> Result<(), String> {
        for key in [
            "MODULE_CONFIG_UDP_FOLLOW_NODE_TCP_PLUS_ONE",
            "MODULE_CONFIG_FIBREENABLED",
        ] {
            if std::env::var(key).is_ok() {
                return Err(retired_fibre_message());
            }
        }
        if !path.exists() {
            return Ok(());
        }
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let value: toml::Value = toml::from_str(&text).map_err(|error| error.to_string())?;
        let Some(table) = value.as_table() else {
            return Ok(());
        };
        if table.contains_key("udp_follow_node_tcp_plus_one") {
            return Err(retired_fibre_message());
        }
        if table
            .get("fibre")
            .and_then(|value| value.as_table())
            .is_some_and(|fibre| fibre.contains_key("enabled"))
        {
            return Err(retired_fibre_message());
        }
        Ok(())
    }

    /// Effective UDP listen address.
    ///
    /// Unset `udp_bind` uses `MODULE_CONFIG_NODE_P2P_LISTEN_*` and adds one to the port.
    pub fn resolve_udp_listen_addr(&self) -> Result<SocketAddr, std::io::Error> {
        use std::io::{Error, ErrorKind};

        match self.udp_bind.as_deref().map(str::trim) {
            None => node_p2p_plus_one(),
            Some("") => Err(Error::new(
                ErrorKind::InvalidInput,
                "blvm-fibre: udp_bind is empty. Omit it to listen on the node P2P port plus one, or set a socket address",
            )),
            Some(value) => SocketAddr::from_str(value).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("blvm-fibre: invalid udp_bind {value:?}: {error}"),
                )
            }),
        }
    }

    /// Key-value map for module context / diagnostics.
    pub fn to_context_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        if let Some(bind) = &self.udp_bind {
            m.insert("blvm-fibre.udp_bind".to_string(), bind.clone());
        }
        m.insert(
            "blvm-fibre.fec_parity_ratio".to_string(),
            format!("{}", self.fibre.fec_parity_ratio),
        );
        m.insert(
            "blvm-fibre.chunk_timeout_secs".to_string(),
            format!("{}", self.fibre.chunk_timeout_secs),
        );
        m.insert(
            "blvm-fibre.fibre_peers_count".to_string(),
            format!("{}", self.fibre_peers.len()),
        );
        m.insert(
            "blvm-fibre.register_peers_from_p2p".to_string(),
            self.register_peers_from_p2p.to_string(),
        );
        m
    }
}

fn node_p2p_plus_one() -> Result<SocketAddr, std::io::Error> {
    use std::io::{Error, ErrorKind};

    let port_s = std::env::var("MODULE_CONFIG_NODE_P2P_LISTEN_PORT").map_err(|_| {
        Error::new(
            ErrorKind::NotFound,
            "blvm-fibre: an omitted udp_bind requires MODULE_CONFIG_NODE_P2P_LISTEN_PORT \
             (set automatically when the node spawns this module)",
        )
    })?;
    let tcp_port: u16 = port_s.parse().map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("blvm-fibre: invalid MODULE_CONFIG_NODE_P2P_LISTEN_PORT: {error}"),
        )
    })?;
    let ip_s = std::env::var("MODULE_CONFIG_NODE_P2P_LISTEN_IP")
        .unwrap_or_else(|_| String::from("0.0.0.0"));
    let ip: IpAddr = ip_s.parse().map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("blvm-fibre: invalid MODULE_CONFIG_NODE_P2P_LISTEN_IP: {error}"),
        )
    })?;
    Ok(SocketAddr::new(ip, tcp_port.saturating_add(1)))
}

fn retired_fibre_message() -> String {
    "udp_follow_node_tcp_plus_one and fibre.enabled are not blvm-fibre settings. Omit udp_bind to listen on the node P2P port plus one, or set udp_bind to a socket address. Loading the module binds UDP".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn omitted_udp_bind_follows_the_node_and_a_value_wins() {
        let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
        unsafe {
            std::env::set_var("MODULE_CONFIG_NODE_P2P_LISTEN_PORT", "8333");
            std::env::set_var("MODULE_CONFIG_NODE_P2P_LISTEN_IP", "127.0.0.1");
        }
        let followed = FibreModuleConfig::default()
            .resolve_udp_listen_addr()
            .unwrap();
        assert_eq!(followed.to_string(), "127.0.0.1:8334");

        let explicit = FibreModuleConfig {
            udp_bind: Some("192.0.2.10:9000".into()),
            ..FibreModuleConfig::default()
        };
        assert_eq!(
            explicit.resolve_udp_listen_addr().unwrap().to_string(),
            "192.0.2.10:9000"
        );
        let empty = FibreModuleConfig {
            udp_bind: Some("  ".into()),
            ..FibreModuleConfig::default()
        };
        assert!(empty.resolve_udp_listen_addr().is_err());
        unsafe {
            std::env::remove_var("MODULE_CONFIG_NODE_P2P_LISTEN_PORT");
            std::env::remove_var("MODULE_CONFIG_NODE_P2P_LISTEN_IP");
        }
    }

    #[test]
    fn a_retired_follow_flag_fails() {
        let dir = std::env::temp_dir().join(format!("fibre-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "udp_follow_node_tcp_plus_one = true\n").unwrap();
        let error = FibreModuleConfig::reject_retired(&path).unwrap_err();
        assert!(error.contains("udp_bind"), "{error}");
        std::fs::write(&path, "[fibre]\nenabled = false\n").unwrap();
        assert!(FibreModuleConfig::reject_retired(&path).is_err());
        std::fs::write(&path, "udp_bind = \"127.0.0.1:8334\"\n").unwrap();
        assert!(FibreModuleConfig::reject_retired(&path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
