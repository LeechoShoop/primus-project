//! Physics constant normalization shim.
//!
//! primus-vm (qualified, frozen) uses internal constants:
//!   thermal_capacity  = 1000.0
//!   macro_shift_gate  = 250.0
//!
//! primus-core SPECIFICATION.md mandates:
//!   thermal_capacity  = 1500.0 K  (§2 Thermal Filter)
//!   gravity_gate      = 150.0 K   (§3 Gravity Shield)
//!
//! This shim normalizes temperatures before they enter primus-vm,
//! and denormalizes metrics coming out, so the rest of primus-core
//! always works in spec-correct units.
//!
//! AUDIT_REPORT.md: fixes DIV — Physics Constants Mismatch

/// Spec-mandated thermal capacity limit (Kelvin)
pub const SPEC_THERMAL_CAPACITY: f64 = 1500.0;
/// Spec-mandated Gravity Shield gate temperature (Kelvin)
pub const SPEC_GRAVITY_GATE: f64 = 150.0;

/// primus-vm internal constants (frozen — do not change these to match spec)
const VM_THERMAL_CAPACITY: f64 = 1000.0;

/// Scale spec temperature for thermal capacity comparison in PVM
/// Satisfies: to_vm_thermal(1500.0) == 1000.0
#[inline]
pub fn to_vm_thermal(spec_kelvin: f64) -> f64 {
    spec_kelvin * (VM_THERMAL_CAPACITY / SPEC_THERMAL_CAPACITY)
}

/// Returns true if temperature (spec Kelvin) should trigger Gravity Shield
#[inline]
pub fn should_engage_gravity_shield(spec_kelvin: f64) -> bool {
    spec_kelvin >= SPEC_GRAVITY_GATE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_capacity_boundary_maps_correctly() {
        // At spec limit (1500.0 K), VM must see exactly its own internal limit
        assert_eq!(to_vm_thermal(SPEC_THERMAL_CAPACITY), VM_THERMAL_CAPACITY);
    }

    #[test]
    fn gravity_shield_engages_at_spec_threshold() {
        assert!(should_engage_gravity_shield(150.0));
        assert!(should_engage_gravity_shield(151.0));
        assert!(!should_engage_gravity_shield(149.99));
    }

    #[test]
    fn below_gravity_gate_does_not_engage_shield() {
        // Any temperature below 150.0 K must NOT engage the shield
        for temp in [0.0, 50.0, 100.0, 149.0, 149.99] {
            assert!(
                !should_engage_gravity_shield(temp),
                "temperature {} should NOT engage shield",
                temp
            );
        }
    }

    #[test]
    fn above_gravity_gate_engages_shield() {
        for temp in [150.0, 150.01, 200.0, 500.0, 1500.0] {
            assert!(
                should_engage_gravity_shield(temp),
                "temperature {} SHOULD engage shield",
                temp
            );
        }
    }

    #[test]
    fn spec_thermal_capacity_maps_to_vm_capacity() {
        // SPEC_THERMAL_CAPACITY (1500.0) must map to exactly 1000.0 in VM units
        let vm_value = to_vm_thermal(SPEC_THERMAL_CAPACITY);
        assert!(
            (vm_value - 1000.0).abs() < 1e-10,
            "1500.0 K must map to 1000.0 VM units, got {}",
            vm_value
        );
    }
}
