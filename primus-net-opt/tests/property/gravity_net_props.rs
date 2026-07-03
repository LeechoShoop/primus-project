use primus_net_opt::gravity_net::{get_galactic_drift, is_resonant};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    #[test]
    fn galactic_drift_bounded(crystal_index: u64) {
        let drift = get_galactic_drift(crystal_index);
        prop_assert!(drift <= 255u8);
    }

    #[test]
    fn resonance_is_deterministic(byte: u8, index: u64) {
        prop_assert_eq!(
            is_resonant(byte, index),
            is_resonant(byte, index),
        );
    }

    #[test]
    fn resonance_period_is_256(byte: u8, index: u64) {
        // is_resonant is periodic with period 256 in crystal_index
        prop_assert_eq!(
            is_resonant(byte, index),
            is_resonant(byte, index.wrapping_add(256)),
        );
    }
}
