//! FIBRE UDP/FEC relay as a **`blvm-node`** loadable module.

pub mod config;
pub mod module;
pub mod relay;
pub mod wire;

pub use config::{FibreModuleConfig, FibrePeerEntry};
pub use module::FibreModule;
pub use relay::{
    start_chunk_processor, FibreConfig, FibreError, FibreRelay, FibreStats,
};
