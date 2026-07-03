#![no_main]

use libfuzzer_sys::fuzz_target;
use primus_net_opt::gravity_shield::GravityShield;

fuzz_target!(|data: &[u8]| {
    let shield = GravityShield::new();
    let _ = shield.filter_bytes(data);
    // Must never panic.
    // Drop counter must not overflow (u64 max ≈ 1.8×10¹⁹).
});
