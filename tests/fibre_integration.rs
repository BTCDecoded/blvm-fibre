//! Integration and wire tests for **`blvm-fibre`** (moved from `blvm-node` / `blvm-protocol`).

use blvm_fibre::wire::{
    FecChunk, FibreCapabilities, FibreConfig, FibreProtocolError, DEFAULT_SHARD_SIZE, FIBRE_MAGIC,
    HEADER_SIZE, MAX_DATA_SIZE,
};
use blvm_fibre::FibreRelay;
use blvm_protocol::{Block, BlockHeader};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

fn create_test_block() -> Block {
    Block {
        header: BlockHeader {
            version: 1,
            prev_block_hash: [0x11; 32],
            merkle_root: [0x22; 32],
            timestamp: 1234567890,
            bits: 0x1d00ffff,
            nonce: 0x12345678,
        },
        transactions: vec![].into(),
    }
}

#[tokio::test]
async fn test_fibre_relay_encode_decode_cycle() {
    let mut relay = FibreRelay::new();
    let block = create_test_block();

    let encoded = relay.encode_block(block.clone()).unwrap();
    assert!(encoded.chunk_count > 0);

    let cached = relay.get_encoded_block(&encoded.block_hash);
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().block_hash, encoded.block_hash);
}

#[tokio::test]
async fn test_fibre_peer_registration() {
    let mut relay = FibreRelay::new();
    let udp_addr: SocketAddr = "127.0.0.1:8334".parse().unwrap();

    relay.register_fibre_peer("peer1".to_string(), Some(udp_addr));
    relay.register_fibre_peer("peer2".to_string(), None);

    assert!(relay.is_fibre_peer("peer1"));
    assert!(relay.is_fibre_peer("peer2"));
    assert!(!relay.is_fibre_peer("peer3"));

    let peers = relay.get_fibre_peers();
    assert_eq!(peers.len(), 2);
}

#[tokio::test]
async fn test_fibre_block_assembly() {
    let mut relay = FibreRelay::new();
    let block = create_test_block();

    let encoded = relay.encode_block(block.clone()).unwrap();
    assert!(encoded.chunk_count > 0);

    let mut assembled: Option<Block> = None;
    for chunk in &encoded.chunks {
        let result = relay.process_received_chunk(chunk.clone()).await.unwrap();
        if let Some(b) = result {
            assembled = Some(b);
            break;
        }
    }
    let assembled = assembled.expect("Block should assemble from chunks");
    assert_eq!(
        assembled.header.prev_block_hash,
        block.header.prev_block_hash
    );
    assert_eq!(assembled.header.merkle_root, block.header.merkle_root);
}

#[tokio::test]
async fn test_fibre_fec_recovery() {
    let config = FibreConfig {
        enabled: true,
        fec_parity_ratio: 0.5,
        chunk_timeout_secs: 2,
        max_retries: 3,
        max_assemblies: 10,
    };
    let mut relay = FibreRelay::with_config(config);
    let block = create_test_block();

    let encoded = relay.encode_block(block.clone()).unwrap();
    assert!(
        encoded.chunk_count >= 2,
        "Need multiple chunks for FEC test"
    );

    let chunks_to_send: Vec<_> = encoded
        .chunks
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 0)
        .map(|(_, c)| c.clone())
        .collect();

    let mut assembled: Option<Block> = None;
    for chunk in chunks_to_send {
        let result = relay.process_received_chunk(chunk).await.unwrap();
        if let Some(b) = result {
            assembled = Some(b);
            break;
        }
    }
    let assembled = assembled.expect("FEC should recover block with 1 missing data chunk");
    assert_eq!(
        assembled.header.prev_block_hash,
        block.header.prev_block_hash
    );
}

#[tokio::test]
async fn test_fibre_chunk_serialization_roundtrip() {
    let block_hash = [0x42; 32];
    let data = vec![1, 2, 3, 4, 5];

    let chunk = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: data.clone(),
        size: data.len(),
        block_hash,
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };
    let packet = chunk.serialize().unwrap();

    let chunk = FecChunk::deserialize(&packet).unwrap();
    assert_eq!(chunk.index, 0);
    assert_eq!(chunk.total_chunks, 10);
    assert_eq!(chunk.data_chunks, 8);
    assert_eq!(chunk.data, data);
    assert_eq!(chunk.block_hash, block_hash);
    assert_eq!(chunk.sequence, 12345);

    let reserialized = chunk.serialize().unwrap();
    assert_eq!(reserialized, packet);
}

#[tokio::test]
async fn test_fibre_two_node_block_relay() {
    let block = create_test_block();

    let mut relay_b = FibreRelay::new();
    let chunk_rx_b = relay_b
        .initialize_udp("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let relay_b_addr = relay_b
        .udp_local_addr()
        .await
        .expect("B should have UDP transport");

    let mut relay_a = FibreRelay::new();
    relay_a
        .initialize_udp("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    relay_a.register_fibre_peer("node_b".to_string(), Some(relay_b_addr));

    let encoded = relay_a.encode_block(block.clone()).unwrap();
    relay_a.send_block("node_b", encoded).await.unwrap();

    let relay_b_arc = Arc::new(Mutex::new(relay_b));
    let relay_for_task = Arc::clone(&relay_b_arc);
    let ingest = tokio::spawn(async move {
        let mut rx = chunk_rx_b;
        while let Some((_addr, chunk)) = rx.recv().await {
            let mut g = relay_for_task.lock().await;
            let _ = g.process_received_chunk(chunk).await;
        }
    });

    sleep(Duration::from_millis(200)).await;

    ingest.abort();
    let _ = ingest.await;

    let relay_b_guard = relay_b_arc.lock().await;
    let stats = relay_b_guard.get_stats().await;
    assert!(
        stats.blocks_received >= 1 || stats.chunks_received >= 1,
        "Node B should receive block or chunks: blocks_received={}, chunks_received={}",
        stats.blocks_received,
        stats.chunks_received
    );
}

#[test]
fn test_fec_chunk_edge_cases() {
    let max_data = vec![0u8; MAX_DATA_SIZE];
    let chunk = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: max_data.clone(),
        size: max_data.len(),
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };

    let serialized = chunk.serialize().unwrap();
    assert!(serialized.len() >= HEADER_SIZE + MAX_DATA_SIZE + 4);

    let deserialized = FecChunk::deserialize(&serialized).unwrap();
    assert_eq!(deserialized.data.len(), MAX_DATA_SIZE);
}

#[test]
fn test_fec_chunk_empty_data() {
    let chunk = FecChunk {
        index: 0,
        total_chunks: 1,
        data_chunks: 1,
        data: vec![],
        size: 0,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };

    let serialized = chunk.serialize().unwrap();
    let deserialized = FecChunk::deserialize(&serialized).unwrap();
    assert_eq!(deserialized.data.len(), 0);
}

#[test]
fn test_fec_chunk_invalid_version() {
    let chunk = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };

    let mut serialized = chunk.serialize().unwrap();
    serialized[4] = 0xFF;

    let result = FecChunk::deserialize(&serialized);
    assert!(result.is_err());
}

#[test]
fn test_fec_chunk_invalid_packet_type() {
    let chunk = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };

    let mut serialized = chunk.serialize().unwrap();
    serialized[5] = 0xFF;

    let result = FecChunk::deserialize(&serialized);
    assert!(result.is_err());
}

#[test]
fn test_fec_chunk_data_length_mismatch() {
    let chunk = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };

    let mut serialized = chunk.serialize().unwrap();
    serialized[58] = 0xFF;
    serialized[59] = 0xFF;
    serialized[60] = 0xFF;
    serialized[61] = 0xFF;

    let result = FecChunk::deserialize(&serialized);
    assert!(result.is_err());
}

#[test]
fn test_fec_chunk_too_short() {
    let short_data = vec![0u8; HEADER_SIZE + 3];

    let result = FecChunk::deserialize(&short_data);
    assert!(result.is_err());
    if let Err(FibreProtocolError::InvalidPacket(msg)) = result {
        assert!(msg.contains("short") || msg.contains("length"));
    }
}

#[test]
fn test_fibre_config_edge_cases() {
    let config = FibreConfig {
        enabled: true,
        fec_parity_ratio: 0.0,
        chunk_timeout_secs: 1,
        max_retries: 1,
        max_assemblies: 1,
    };
    assert_eq!(config.fec_parity_ratio, 0.0);

    let config = FibreConfig {
        enabled: true,
        fec_parity_ratio: 1.0,
        chunk_timeout_secs: 100,
        max_retries: 10,
        max_assemblies: 100,
    };
    assert_eq!(config.fec_parity_ratio, 1.0);
}

#[test]
fn test_fibre_capabilities_edge_cases() {
    let caps1 = FibreCapabilities {
        supports_fec: true,
        max_chunk_size: DEFAULT_SHARD_SIZE,
        min_latency: true,
    };
    assert!(caps1.supports_fec);

    let caps2 = FibreCapabilities {
        supports_fec: false,
        max_chunk_size: 1000,
        min_latency: false,
    };
    assert!(!caps2.supports_fec);
    assert_eq!(caps2.max_chunk_size, 1000);
}

#[test]
fn test_fec_chunk_index_boundaries() {
    let chunk1 = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };
    let serialized1 = chunk1.serialize().unwrap();
    let deserialized1 = FecChunk::deserialize(&serialized1).unwrap();
    assert_eq!(deserialized1.index, 0);

    let chunk2 = FecChunk {
        index: 9,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 12345,
        magic: FIBRE_MAGIC,
    };
    let serialized2 = chunk2.serialize().unwrap();
    let deserialized2 = FecChunk::deserialize(&serialized2).unwrap();
    assert_eq!(deserialized2.index, 9);
}

#[test]
fn test_fec_chunk_sequence_numbers() {
    let chunk1 = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: 0,
        magic: FIBRE_MAGIC,
    };
    let serialized1 = chunk1.serialize().unwrap();
    let deserialized1 = FecChunk::deserialize(&serialized1).unwrap();
    assert_eq!(deserialized1.sequence, 0);

    let chunk2 = FecChunk {
        index: 0,
        total_chunks: 10,
        data_chunks: 8,
        data: vec![1, 2, 3],
        size: 3,
        block_hash: [0x42; 32],
        sequence: u64::MAX,
        magic: FIBRE_MAGIC,
    };
    let serialized2 = chunk2.serialize().unwrap();
    let deserialized2 = FecChunk::deserialize(&serialized2).unwrap();
    assert_eq!(deserialized2.sequence, u64::MAX);
}
