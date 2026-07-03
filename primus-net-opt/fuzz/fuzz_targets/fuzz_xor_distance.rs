#![no_main]

use libfuzzer_sys::fuzz_target;
use primus_net_opt::dht::{bucket_index, xor_distance};

fuzz_target!(|data: &[u8]| {
    if data.len() >= 64 {
        let a: [u8; 32] = data[..32].try_into().unwrap();
        let b: [u8; 32] = data[32..64].try_into().unwrap();
        let dist = xor_distance(&a, &b);
        let idx = bucket_index(&dist);
        assert!(idx < 256, "bucket_index out of range: {idx}");
    }
});
