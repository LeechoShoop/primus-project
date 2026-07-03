use primus_net_opt::gravity_shield::GravityShield;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    #[test]
    fn drop_counter_monotonically_increases(inputs in proptest::collection::vec(
        proptest::collection::vec(any::<u8>(), 0..1024),
        0..50
    )) {
        let shield = GravityShield::new();
        let mut last = 0u64;
        for raw in &inputs {
            let _ = shield.filter_bytes(raw);
            let current = shield.drop_count();
            prop_assert!(current >= last);
            last = current;
        }
    }
}
