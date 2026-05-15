//! Module config: FIBRE protocol options + UDP bind.

use crate::wire::FibreConfig;
use blvm_sdk_macros::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

fn default_udp_bind() -> String {
    "0.0.0.0:8334".to_string()
}

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
#[config(name = "blvm-fibre")]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FibreModuleConfig {
    /// FIBRE protocol options (FEC, timeouts, etc.).
    #[serde(default)]
    pub fibre: FibreConfig,
    /// UDP listen address (`host:port`).
    #[serde(default = "default_udp_bind")]
    #[config_env]
    pub udp_bind: String,
    /// Outbound FIBRE targets (optional until dynamic registration exists).
    #[serde(default)]
    pub fibre_peers: Vec<FibrePeerEntry>,
    /// Match in-process FIBRE: UDP port = node's P2P TCP listen port + 1.
    ///
    /// Requires `MODULE_CONFIG_NODE_P2P_LISTEN_PORT` and optional `MODULE_CONFIG_NODE_P2P_LISTEN_IP`
    /// (injected when the node spawns the module). When `true`, **`udp_bind`** is ignored for listening.
    #[serde(default)]
    #[config_env]
    pub udp_follow_node_tcp_plus_one: bool,
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
            udp_bind: default_udp_bind(),
            fibre_peers: Vec::new(),
            udp_follow_node_tcp_plus_one: false,
            register_peers_from_p2p: default_register_peers_from_p2p(),
        }
    }
}

blvm_sdk::impl_module_config!(FibreModuleConfig);

impl FibreModuleConfig {
    /// Effective UDP listen address (honours [`Self::udp_follow_node_tcp_plus_one`]).
    pub fn resolve_udp_listen_addr(&self) -> Result<SocketAddr, std::io::Error> {
        use std::io::{Error, ErrorKind};

        if self.udp_follow_node_tcp_plus_one {
            let port_s = std::env::var("MODULE_CONFIG_NODE_P2P_LISTEN_PORT").map_err(|_| {
                Error::new(
                    ErrorKind::NotFound,
                    "blvm-fibre: udp_follow_node_tcp_plus_one requires MODULE_CONFIG_NODE_P2P_LISTEN_PORT \
                     (set automatically when the node spawns this module)",
                )
            })?;
            let tcp_port: u16 = port_s.parse().map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("blvm-fibre: invalid MODULE_CONFIG_NODE_P2P_LISTEN_PORT: {e}"),
                )
            })?;
            let ip_s = std::env::var("MODULE_CONFIG_NODE_P2P_LISTEN_IP")
                .unwrap_or_else(|_| String::from("0.0.0.0"));
            let ip: IpAddr =
                ip_s.parse()
                    .map_err(|e| {
                        Error::new(
                            ErrorKind::InvalidInput,
                            format!("blvm-fibre: invalid MODULE_CONFIG_NODE_P2P_LISTEN_IP: {e}"),
                        )
                    })?;
            let udp_port = tcp_port.saturating_add(1);
            return Ok(SocketAddr::new(ip, udp_port));
        }
        self.udp_socket_addr()
    }

    /// Resolve UDP bind address for [`FibreRelay::initialize_udp`] when using explicit `udp_bind` only.
    pub fn udp_socket_addr(&self) -> Result<SocketAddr, std::io::Error> {
        SocketAddr::from_str(&self.udp_bind).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("blvm-fibre: invalid udp_bind {:?}: {e}", self.udp_bind),
            )
        })
    }

    /// Key-value map for module context / diagnostics.
    pub fn to_context_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("blvm-fibre.udp_bind".to_string(), self.udp_bind.clone());
        m.insert("blvm-fibre.enabled".to_string(), self.fibre.enabled.to_string());
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
            "blvm-fibre.udp_follow_node_tcp_plus_one".to_string(),
            self.udp_follow_node_tcp_plus_one.to_string(),
        );
        m.insert(
            "blvm-fibre.register_peers_from_p2p".to_string(),
            self.register_peers_from_p2p.to_string(),
        );
        m
    }
}
