//! Typed scalar-DP inventory; never emits a proposed allocation.
#[path = "support/movement_inventory.rs"]
mod inventory;

#[test]
#[ignore = "requires A816_COMPARISON_MANIFEST and A816_MOVEMENT_FACTS"]
fn export_verified_scalar_dp_inventory() {
    inventory::export(true);
}
