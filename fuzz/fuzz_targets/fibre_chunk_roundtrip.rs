#![no_main]

use blvm_fibre::wire::FecChunk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(c) = FecChunk::deserialize(data) {
        if let Ok(b) = c.serialize() {
            let _ = FecChunk::deserialize(&b);
        }
    }
});
