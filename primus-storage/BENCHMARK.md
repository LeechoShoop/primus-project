# BENCHMARK.md — primus-storage v0.2.0
**Date:** 2026-05-08
**Machine:** Windows 11 Home, 32GB RAM, ML-DSA-87 Parameter Set

## Fuzz Suite Results

| Test | Iters | Result | Wall time (ms) |
|---|---|---|---|
| fuzz_changeset_deterministic_order | 5 000 | PASS | 487.6 |
| fuzz_changeset_overwrite_semantics | 2 000 | PASS | 258.2 |
| fuzz_changeset_len_consistency | 1 000 | PASS | 241.5 |
| fuzz_mpt_insert_get_roundtrip | 2 000 | PASS | 856.3 |
| fuzz_mpt_root_insertion_order_invariant | 600 | PASS | 1024.1 |
| fuzz_mpt_delete_correctness | 500 | PASS | 412.5 |
| fuzz_inclusion_proof_always_valid | 1 000 | PASS | 562.8 |
| fuzz_exclusion_proof_always_valid | 500 | PASS | 482.1 |
| fuzz_tampered_proof_rejected | ~51 200 | PASS | 3125.4 |
| fuzz_tampered_root_always_rejected | 25 600 | PASS | 1856.2 |
| fuzz_tampered_value_rejected | 50 | PASS | 234.5 |
| fuzz_mpt_node_hash_determinism | 5 000 | PASS | 275.4 |
| fuzz_mpt_node_hash_no_collisions | 5 000 | PASS | 225.5 |
| fuzz_key_to_nibbles_always_64 | 10 000 | PASS | 224.0 |
| fuzz_key_to_nibbles_roundtrip | 10 000 | PASS | 227.0 |
| fuzz_proof_size_budget | 1 000 nodes | PASS | 439.5 |
| fuzz_undo_log_first_write_wins | 2 000 | PASS | 255.7 |
| fuzz_global_metrics_canonical_determinism | 5 000 | PASS | 225.0 |

## Criterion Benchmarks

| Operation | Mean (µs) | Throughput |
|---|---|---|
| mpt_insert (1 node) | 1042.50 | 961 ops/s |
| mpt_insert (100 nodes) | 1134.80 | 881 ops/s |
| mpt_insert (1 000 nodes) | 2195.50 | 455 ops/s |
| mpt_get hit | 4.93 | 202,839 ops/s |
| mpt_get miss | 2.81 | 355,871 ops/s |
| mpt_prove | 5.03 | 198,807 ops/s |
| verify_proof | 2.76 | 362,318 ops/s |
| mpt_delete | 2172.10 | 460 ops/s |
| gc_since | 6088.50 | 164 ops/s |
| changeset_insert ×1 | 0.66 | 1,515,151 ops/s |
| changeset_insert ×1 000 | 639.31 | 1,564 ops/s |
| changeset_get | 0.02 | 38,461,538 ops/s |
| mempool push_bytes | 17.15 | 58,309 ops/s |
| mempool push | 13.12 | 76,219 ops/s |
| node_hash Leaf | 0.09 | 10,526,315 ops/s |
| node_hash Branch | 0.13 | 7,462,686 ops/s |

## Max proof size
| Trie nodes | Max proof bytes |
|---|---|
| 1 000 | 321 |