#![no_main]

use libfuzzer_sys::fuzz_target;
use primus_net_opt::network::PrimusMessage;

fuzz_target!(|data: &[u8]| {
    let _ = bincode::deserialize::<PrimusMessage>(data);
    // Must never panic — anyhow::Error is acceptable.
});
