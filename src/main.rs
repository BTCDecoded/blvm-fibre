//! blvm-fibre — FIBRE UDP/FEC relay (outbound on `NewBlock`, inbound via `queue_received_block_bytes`).

use anyhow::Result;
use blvm_fibre::{FibreModule, FibreModuleConfig, FibreRelay, start_chunk_processor};
use blvm_sdk::module::{ModuleBootstrap, ModuleDb};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

const MODULE_NAME: &str = "blvm-fibre";

#[tokio::main]
async fn main() -> Result<()> {
    let bootstrap = ModuleBootstrap::init_module(MODULE_NAME);
    let db = ModuleDb::open_or_temp(&bootstrap.data_dir, MODULE_NAME)?;

    let setup = |node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
                 _db: Arc<dyn blvm_node::storage::database::Database>,
                 data_dir: &std::path::Path| {
        let bootstrap = bootstrap.clone();
        let data_dir = data_dir.to_path_buf();
        async move {
            let (_ctx, config) = bootstrap.context_with_config::<FibreModuleConfig>(&data_dir);
            if !config.fibre.enabled {
                warn!("blvm-fibre: fibre.enabled is false; UDP and FIBRE relay are off");
                let relay = Arc::new(Mutex::new(FibreRelay::with_config(config.fibre.clone())));
                let module = FibreModule::new(Arc::clone(&relay), false);
                return Ok((module.clone(), module));
            }

            let udp_addr = config
                .resolve_udp_listen_addr()
                .map_err(|e| blvm_node::module::traits::ModuleError::Other(e.to_string()))?;

            let mut relay = FibreRelay::with_config(config.fibre.clone());
            for p in &config.fibre_peers {
                match SocketAddr::from_str(&p.udp_addr) {
                    Ok(addr) => {
                        relay.register_fibre_peer(p.peer_id.clone(), Some(addr));
                        info!("blvm-fibre: static peer {} -> {}", p.peer_id, p.udp_addr);
                    }
                    Err(e) => {
                        warn!(
                            "blvm-fibre: skip peer {:?} (bad udp_addr {:?}): {e}",
                            p.peer_id, p.udp_addr
                        );
                    }
                }
            }

            let chunk_rx = relay.initialize_udp(udp_addr).await.map_err(|e| {
                blvm_node::module::traits::ModuleError::Other(format!("blvm-fibre: UDP init: {e}"))
            })?;

            let relay = Arc::new(Mutex::new(relay));
            let api_ingress = Arc::clone(&node_api);
            let _chunk_processor = start_chunk_processor(Arc::clone(&relay), chunk_rx, api_ingress);

            info!("blvm-fibre: listening UDP {}", udp_addr);

            let module = FibreModule::new(Arc::clone(&relay), config.register_peers_from_p2p);
            Ok((module.clone(), module))
        }
    };

    blvm_sdk::run_module! {
        bootstrap: &bootstrap,
        module_name: MODULE_NAME,
        module_type: FibreModule,
        cli_type: FibreModule,
        db: db.as_db(),
        setup: setup,
        event_types: FibreModule::event_types(),
    }?;

    warn!("blvm-fibre shutting down");
    Ok(())
}
