//! `NewBlock` / `BlockMined` → FIBRE send; received chunks → `NodeAPI::queue_received_block_bytes`.

use blvm_node::module::ipc::protocol::{EventMessage, EventPayload};
use blvm_protocol::Hash;
use blvm_sdk::module::prelude::*;
use blvm_sdk_macros::module;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::relay::FibreRelay;

#[derive(Clone)]
pub struct FibreModule {
    relay: Arc<Mutex<FibreRelay>>,
    register_peers_from_p2p: bool,
}

impl FibreModule {
    pub fn new(relay: Arc<Mutex<FibreRelay>>, register_peers_from_p2p: bool) -> Self {
        Self {
            relay,
            register_peers_from_p2p,
        }
    }

    /// Fetch block from node and send FEC chunks to all registered FIBRE peers (in-process parity).
    async fn send_encoded_block_to_peers(
        &self,
        api: &Arc<dyn NodeAPI>,
        block_hash: &Hash,
        source: &'static str,
    ) -> Result<(), ModuleError> {
        let block = match api.get_block(block_hash).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                debug!(
                    "blvm-fibre: get_block returned None for {:?} ({source})",
                    block_hash
                );
                return Ok(());
            }
            Err(e) => {
                warn!("blvm-fibre: get_block {:?} ({source}): {}", block_hash, e);
                return Ok(());
            }
        };

        let encoded = {
            let mut relay = self.relay.lock().await;
            match relay.encode_block(block) {
                Ok(enc) => enc,
                Err(e) => {
                    warn!("blvm-fibre: encode_block ({source}): {}", e);
                    return Ok(());
                }
            }
        };

        let peer_ids: Vec<String> = {
            let relay = self.relay.lock().await;
            relay
                .get_fibre_peers()
                .iter()
                .map(|p| p.peer_id.clone())
                .collect()
        };

        if peer_ids.is_empty() {
            debug!(
                "blvm-fibre: no FIBRE peers registered; skip send for {:?} ({source})",
                block_hash
            );
            return Ok(());
        }

        for peer_id in peer_ids {
            let mut relay = self.relay.lock().await;
            if let Err(e) = relay.send_block(&peer_id, encoded.clone()).await {
                warn!("blvm-fibre: send_block to {} ({source}): {}", peer_id, e);
            }
        }
        Ok(())
    }
}

#[module]
impl FibreModule {
    #[on_event(NewBlock)]
    async fn on_new_block(
        &self,
        event: &EventMessage,
        ctx: &InvocationContext,
    ) -> Result<(), ModuleError> {
        let api = ctx
            .node_api()
            .ok_or_else(|| ModuleError::Other("blvm-fibre: node_api required".to_string()))?;
        if let EventPayload::NewBlock { block_hash, .. } = event.payload {
            self.send_encoded_block_to_peers(&api, &block_hash, "NewBlock")
                .await?;
        }
        Ok(())
    }

    /// Same outbound path as `NewBlock` when the node mines a block (parity with optional `broadcast` hooks).
    #[on_event(BlockMined)]
    async fn on_block_mined(
        &self,
        event: &EventMessage,
        ctx: &InvocationContext,
    ) -> Result<(), ModuleError> {
        let api = ctx
            .node_api()
            .ok_or_else(|| ModuleError::Other("blvm-fibre: node_api required".to_string()))?;
        if let EventPayload::BlockMined { block_hash, .. } = event.payload {
            self.send_encoded_block_to_peers(&api, &block_hash, "BlockMined")
                .await?;
        }
        Ok(())
    }

    /// P2P peer advertised `NODE_FIBRE`; node emits [`EventPayload::CompanionUdpPeerRegistered`].
    #[on_event(CompanionUdpPeerRegistered)]
    async fn on_companion_udp_peer_registered(
        &self,
        event: &EventMessage,
        _ctx: &InvocationContext,
    ) -> Result<(), ModuleError> {
        if !self.register_peers_from_p2p {
            return Ok(());
        }
        if let EventPayload::CompanionUdpPeerRegistered {
            p2p_peer_addr,
            udp_addr,
        } = &event.payload
        {
            let udp = match SocketAddr::from_str(udp_addr) {
                Ok(a) => a,
                Err(e) => {
                    warn!(
                        "blvm-fibre: CompanionUdpPeerRegistered bad udp_addr {:?}: {e}",
                        udp_addr
                    );
                    return Ok(());
                }
            };
            let mut relay = self.relay.lock().await;
            relay.register_fibre_peer(p2p_peer_addr.clone(), Some(udp));
            debug!(
                "blvm-fibre: P2P peer {} -> FIBRE UDP {} (dynamic)",
                p2p_peer_addr, udp_addr
            );
        }
        Ok(())
    }

    #[on_event(CompanionUdpPeerUnregistered)]
    async fn on_companion_udp_peer_unregistered(
        &self,
        event: &EventMessage,
        _ctx: &InvocationContext,
    ) -> Result<(), ModuleError> {
        if !self.register_peers_from_p2p {
            return Ok(());
        }
        if let EventPayload::CompanionUdpPeerUnregistered { p2p_peer_addr } = &event.payload {
            let mut relay = self.relay.lock().await;
            if relay.unregister_fibre_peer(p2p_peer_addr.as_str()) {
                debug!("blvm-fibre: removed dynamic FIBRE peer {}", p2p_peer_addr);
            }
        }
        Ok(())
    }

    #[command]
    fn help(&self, _ctx: &InvocationContext) -> Result<String, ModuleError> {
        Ok(
            "blvm-fibre — UDP/FEC block relay for blvm-node.\n\
             Configure module `config.toml` / `[modules.blvm-fibre]` (FibreConfig + udp_bind or udp_follow_node_tcp_plus_one).\n\
             Node injects MODULE_CONFIG_NODE_P2P_LISTEN_* on spawn when using udp_follow_node_tcp_plus_one (UDP = TCP+1 like in-process FIBRE).\n\
             Capabilities: read_blockchain, subscribe_events, queue_inbound_block.\n\
             Optional `[[fibre_peers]]`; `register_peers_from_p2p` subscribes to CompanionUdpPeer* for NODE_FIBRE peers."
                .to_string(),
        )
    }
}
