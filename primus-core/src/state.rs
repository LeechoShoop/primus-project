// =============================================================================
// state.rs — Deterministic State Tree (Mainnet-Ready)
//
// CHANGES vs previous revision (State Root Mismatch fix):
//
//   BUG 3 FIX — Changeset::inner is now BTreeMap<Vec<u8>, PrimusAtom>.
//     The previous HashMap gave non-deterministic iteration order across
//     process instances and CPU architectures. BTreeMap enforces lexicographic
//     key order, so every node iterates the changeset in the same sequence.
//
//   BUG 2 FIX — Fixed-point physics helpers added.
//     f32/f64 arithmetic is not bit-exact across x86 (80-bit extended
//     precision FPU) and ARM (strict 32-bit). Any f32 value that participates
//     in the state root hash must first be converted to a canonical u64 via
//     PhysicsCanon::encode(). See the PhysicsCanon helper below.
//
//   ROOT HASH HARDENING — calculate_root_hash() now feeds u64 fixed-point
//     representations of GlobalMetrics into the hasher so two nodes with
//     logically identical temperature/entropy always produce the same root,
//     even if their f32 register values differ by 1 ULP.
//
// INVARIANTS (do not break):
//   1. Changeset always iterates in BTreeMap key order (Vec<u8> lex order).
//   2. calculate_root_hash() must be pure: identical in-memory state =>
//      identical 32-byte output, regardless of hardware or OS.
//   3. PhysicsCanon::encode() is the SOLE path for f32 -> hash input.
//      Never pass raw f32 bits to a hasher anywhere in the codebase.
// =============================================================================

use primus_types::atom::Atom;
pub use primus_types::physics::PhysicsCanon;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use primus_storage::{Changeset, GlobalMetrics};

// ── StateTree ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StateTree {
    /// Global atom registry (Hot Storage).
    /// BTreeMap ensures deterministic iteration for root hash calculation.
    pub atoms: BTreeMap<Vec<u8>, Atom>,

    /// Current block height.
    pub current_crystal_index: u64,

    /// Thermodynamic state persisted across restarts.
    pub global_metrics: GlobalMetrics,
}

impl StateTree {
    pub fn new() -> Self {
        Self {
            atoms: BTreeMap::new(),
            current_crystal_index: 0,
            global_metrics: GlobalMetrics::default(),
        }
    }

    pub fn load(&mut self, atoms_from_storage: BTreeMap<Vec<u8>, Atom>, metrics: GlobalMetrics) {
        let count = atoms_from_storage.len();
        self.atoms = atoms_from_storage;
        self.global_metrics = metrics;
        println!(
            "🧠 StateTree: Loaded {} atoms. Temp: {:.2}, Entropy: {:.2}",
            count, self.global_metrics.temperature, self.global_metrics.entropy,
        );
    }

    pub fn get_atom(&self, pubkey: &[u8]) -> Option<&Atom> {
        self.atoms.get(pubkey)
    }

    pub fn apply_changeset(&mut self, changeset: Changeset) {
        for (pk, atom) in changeset.inner {
            self.atoms.insert(pk, atom);
        }
    }

    #[allow(dead_code)]
    pub fn increment_index(&mut self) {
        self.current_crystal_index += 1;
    }

    // =========================================================================
    // DETERMINISTIC STATE ROOT
    // =========================================================================

    /// Calculate a canonical state root over every atom in the tree.
    ///
    /// # Determinism guarantees
    ///
    /// 1. Key order: `self.atoms` is a `BTreeMap`, so iteration is always
    ///    in lexicographic public-key order — identical on every node.
    ///
    /// 2. Atom serialisation: `bincode::serialize` is deterministic for
    ///    fixed-layout structs. All integer fields are little-endian. f32
    ///    fields (charge) are serialised as their raw IEEE 754 bits by
    ///    bincode. `charge` is set from constants (never computed cross-node)
    ///    and is therefore safe. If charge ever becomes computed, run it
    ///    through PhysicsCanon first.
    ///
    /// 3. Physics metrics: temperature and entropy are encoded via
    ///    PhysicsCanon::encode() before being fed to the hasher, collapsing
    ///    any 1-ULP f32 differences that could arise from differing FPU modes.
    ///
    /// 4. Hash function: BLAKE3 is a Merkle tree construction and is
    ///    platform-independent. Output is always a 32-byte array.
    pub fn calculate_root_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();

        // 1. Block height — binds the root to a specific chain position.
        hasher.update(&self.current_crystal_index.to_le_bytes());

        // 2. Physics metrics in fixed-point form — BUG 2 FIX.
        let (temp_fixed, entropy_fixed) = self.global_metrics.canonical();
        hasher.update(&temp_fixed.to_le_bytes());
        hasher.update(&entropy_fixed.to_le_bytes());

        // 3. All atoms in deterministic key order — BUG 3 FIX (BTreeMap).
        //    `self.atoms` is already a BTreeMap; no re-sort needed.
        for (pk, atom) in &self.atoms {
            hasher.update(pk);
            if let Ok(bytes) = bincode::serialize(atom) {
                hasher.update(&bytes);
            }
        }

        *hasher.finalize().as_bytes()
    }
}

// =============================================================================
// STATE DIFF DEBUGGER
// =============================================================================

/// Canonical hex representation of a single atom for cross-node comparison.
///
/// Format: "PUBKEY_HEX | mass=NNN | elem=X | charge_fixed=NNN | last_rx=HEX"
///
/// `charge_fixed` uses PhysicsCanon so the representation is bit-identical
/// across hardware.
#[allow(dead_code)]
pub fn atom_canonical_line(pk: &[u8], atom: &Atom) -> String {
    use crate::atom::Element;
    let elem_str = match atom.element {
        Element::Hydrogen => "H",
        Element::Carbon => "C",
        Element::Oxygen => "O",
        Element::Gold => "Au",
    };
    format!(
        "{} | mass={:>20} | elem={} | charge_fixed={:>15} | last_rx={}",
        hex::encode(pk),
        atom.mass,
        elem_str,
        PhysicsCanon::encode(atom.charge),
        hex::encode(&atom.last_reaction_hash[..8]),
    )
}

/// Print a full canonical diff of two state trees to stdout.
///
/// Call this on both nodes with the same Crystal index to pinpoint exactly
/// which atom(s) differ and why. Each diverging atom is printed with its
/// full canonical line from both nodes so you can see the exact field(s)
/// that differ.
///
/// # Usage
///
/// ```rust
/// // In a debug RPC endpoint or unit test:
/// let node0_state: StateTree = ...;
/// let node1_state: StateTree = ...;
/// state_diff(&node0_state, &node1_state, crystal_index);
/// ```
#[allow(dead_code)]
pub fn state_diff(node_a: &StateTree, node_b: &StateTree, at_index: u64) {
    println!("===================================================================");
    println!("  STATE DIFF at Crystal #{}", at_index);
    println!(
        "  Node-A root: {}",
        hex::encode(node_a.calculate_root_hash())
    );
    println!(
        "  Node-B root: {}",
        hex::encode(node_b.calculate_root_hash())
    );
    println!("-------------------------------------------------------------------");

    // Physics metrics comparison (in canonical fixed-point)
    let (a_temp, a_ent) = node_a.global_metrics.canonical();
    let (b_temp, b_ent) = node_b.global_metrics.canonical();
    if a_temp != b_temp || a_ent != b_ent {
        println!("  METRIC DIVERGENCE:");
        println!(
            "      Temp    A={} B={} delta={}",
            a_temp,
            b_temp,
            a_temp.abs_diff(b_temp)
        );
        println!(
            "      Entropy A={} B={} delta={}",
            a_ent,
            b_ent,
            a_ent.abs_diff(b_ent)
        );
    } else {
        println!(
            "  Metrics match (temp_fixed={} entropy_fixed={})",
            a_temp, a_ent
        );
    }
    println!("-------------------------------------------------------------------");

    let mut diverged = 0usize;

    // Atoms in A — check against B
    for (pk, atom_a) in &node_a.atoms {
        match node_b.atoms.get(pk) {
            None => {
                println!("  ONLY IN A: {}", atom_canonical_line(pk, atom_a));
                diverged += 1;
            }
            Some(atom_b) => {
                let line_a = atom_canonical_line(pk, atom_a);
                let line_b = atom_canonical_line(pk, atom_b);
                if line_a != line_b {
                    println!("  DIVERGED:");
                    println!("      A: {}", line_a);
                    println!("      B: {}", line_b);
                    diverged += 1;
                }
            }
        }
    }

    // Atoms present in B but absent in A
    for (pk, atom_b) in &node_b.atoms {
        if !node_a.atoms.contains_key(pk) {
            println!("  ONLY IN B: {}", atom_canonical_line(pk, atom_b));
            diverged += 1;
        }
    }

    println!("-------------------------------------------------------------------");
    if diverged == 0 {
        println!("  All {} atom(s) match.", node_a.atoms.len());
    } else {
        println!(
            "  {} atom(s) diverged. Heights: A={} B={}",
            diverged, node_a.current_crystal_index, node_b.current_crystal_index,
        );
    }
    println!("===================================================================");
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::{Atom, Element};

    fn make_atom(pk: Vec<u8>, mass: u64) -> Atom {
        let mut a = Atom::new_materialized(pk, Element::Hydrogen);
        a.mass = mass;
        a
    }

    #[test]
    fn identical_states_produce_identical_root() {
        let mut s1 = StateTree::new();
        let mut s2 = StateTree::new();
        let pk = vec![1, 2, 3, 4];
        s1.atoms.insert(pk.clone(), make_atom(pk.clone(), 100_000));
        s2.atoms.insert(pk.clone(), make_atom(pk.clone(), 100_000));
        assert_eq!(s1.calculate_root_hash(), s2.calculate_root_hash());
    }

    #[test]
    fn mass_difference_changes_root() {
        let mut s1 = StateTree::new();
        let mut s2 = StateTree::new();
        let pk = vec![5, 6, 7, 8];
        s1.atoms.insert(pk.clone(), make_atom(pk.clone(), 100_000));
        s2.atoms.insert(pk.clone(), make_atom(pk.clone(), 100_001));
        assert_ne!(s1.calculate_root_hash(), s2.calculate_root_hash());
    }

    #[test]
    fn changeset_iteration_is_deterministic() {
        let mut cs = Changeset::new();
        cs.insert(vec![0xcc], make_atom(vec![0xcc], 1));
        cs.insert(vec![0xaa], make_atom(vec![0xaa], 2));
        cs.insert(vec![0xbb], make_atom(vec![0xbb], 3));
        // BTreeMap guarantees lexicographic order regardless of insertion order
        let keys: Vec<_> = cs.sorted_keys().collect();
        assert_eq!(keys, vec![&vec![0xaa], &vec![0xbb], &vec![0xcc]]);
    }

    #[test]
    fn physics_canon_round_trip() {
        let original = 137.42_f32;
        let encoded = PhysicsCanon::encode(original);
        let decoded = PhysicsCanon::decode(encoded);
        // Round-trip precision within 10^-6
        assert!(
            (original - decoded).abs() < 1e-6_f32,
            "round-trip error: {} -> {} -> {}",
            original,
            encoded,
            decoded
        );
    }

    #[test]
    fn state_diff_no_divergence_for_identical() {
        let mut s = StateTree::new();
        s.atoms.insert(vec![1], make_atom(vec![1], 42));
        // Should not panic and should print "All 1 atom(s) match."
        state_diff(&s, &s.clone(), 1);
    }
}
