#![no_main]

use blvm_fibre::wire::FecChunk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| { let _ = FecChunk::deserialize(data); });
