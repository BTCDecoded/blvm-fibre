# blvm-fibre

First-party **blvm-node** module that relays **blocks** over UDP with Reed–Solomon FEC:

| Direction | When |
|-----------|------|
| Outbound | `NewBlock` / `BlockMined` → `NodeAPI::get_block` → FEC chunks to registered peers |
| Inbound | Assembled chunk bytes → `NodeAPI::queue_received_block_bytes` |

Wire types (`FecChunk`, `FibreConfig`, `FIBRE_MAGIC`, CRC32) live in `src/wire.rs`. After UDP bind succeeds, the module asks the node to advertise `NODE_FIBRE` (`1 << 26`). The bit stays set only while that module is still loaded. Unload and crash leave the bit clear.

Former in-process FIBRE was **removed from `blvm-node`**; use this module instead. The node still publishes a companion UDP address (peer TCP port + 1) when a remote `version` has `NODE_FIBRE`.

## Configure

In the module data directory (or node overrides under `[modules.blvm-fibre]`):

```toml
register_peers_from_p2p = true

[[fibre_peers]]
peer_id = "relay-east"
udp_addr = "203.0.113.10:8334"
```

Omit `udp_bind` to listen on the node P2P address with the port plus one (`MODULE_CONFIG_NODE_P2P_LISTEN_*` is injected on spawn). Set `udp_bind` to a `host:port` to listen there. An empty `udp_bind` is an error. `register_peers_from_p2p` (default true) adds peers from `CompanionUdpPeerRegistered` / `Unregistered`. `[[fibre_peers]]` is the static outbound list (`peer_id`, `udp_addr`). Nested `[fibre]` is `FibreConfig` in `src/wire.rs` (`fec_parity_ratio`, `chunk_timeout_secs`, `max_retries`, `max_assemblies`).

## Build (monorepo)

```bash
cd blvm-fibre && cargo build
```

Patches in `Cargo.toml` point at sibling `blvm-*` crates; strip them for crates.io-only CI.

## Install

Copy the binary + `module.toml` into `modules/blvm-fibre/`. There is no registry bootstrap row while `[downloads]` is empty.

## Releases and `[downloads]`

`module.toml` has no **`[downloads]`** table yet. Empty until a release is cut — treat as **non-bootstrap** until filled. Policy: [Module registry](../../blvm/README.md#module-registry) in the **`blvm`** crate README.
