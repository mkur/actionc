//! Read-only facts for the post-scalar-DP machine-code inventory.
#[path = "support/movement_inventory.rs"]
mod inventory;

#[test]
#[ignore = "requires A816_COMPARISON_MANIFEST and A816_MOVEMENT_FACTS"]
fn export_verified_register_inventory() {
    inventory::export_registers();
}
