//! Independent host-side oracle for the packed layout in DIR.ACT.
//! Keep these offsets explicit so a compiler layout error cannot update the
//! provider fixture and its expected results together.

pub const FIRST: u16 = 0;
pub const COUNT: u16 = 2;
pub const VALID: u16 = 3;
pub const ENDED: u16 = 4;
pub const ORDER: u16 = 5;
pub const SUMMARY: u16 = 69;
pub const ITEMS: u16 = 87;
pub const ENTRY_SIZE: u16 = 20;
pub const BATCH_SIZE: u16 = 1367;
pub const CAPACITY: u16 = 64;
