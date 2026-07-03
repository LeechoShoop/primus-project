use primus_net_opt::dht::{NBUCKETS, bucket_index, xor_distance};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    #[test]
    fn xor_reflexive(a: [u8; 32]) {
        prop_assert_eq!(xor_distance(&a, &a), [0u8; 32]);
    }

    #[test]
    fn xor_symmetric(a: [u8; 32], b: [u8; 32]) {
        prop_assert_eq!(xor_distance(&a, &b), xor_distance(&b, &a));
    }

    #[test]
    fn xor_triangle_inequality(a: [u8; 32], b: [u8; 32], c: [u8; 32]) {
        // XOR metric satisfies ultrametric inequality:
        // dist(a,c) <= max(dist(a,b), dist(b,c))
        let ac = xor_distance(&a, &c);
        let ab = xor_distance(&a, &b);
        let bc = xor_distance(&b, &c);
        let max_ab_bc: Vec<u8> = ab.iter().zip(bc.iter())
            .map(|(x, y)| x | y)
            .collect();
        prop_assert!(ac.as_slice() <= max_ab_bc.as_slice());
    }

    #[test]
    fn bucket_index_in_range(a: [u8; 32], b: [u8; 32]) {
        let dist = xor_distance(&a, &b);
        let idx = bucket_index(&dist);
        prop_assert!(idx < NBUCKETS);
    }
}
