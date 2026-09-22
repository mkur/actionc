//! Independent byte-identity namespace for word oracles: 1..254 are S-relative,
//! 256+offset is D-relative. The gap prevents cross-space overlap arithmetic.
//! These test identities are never serialized as compiler allocation locations.
use actionc::mir65816::emit::Location;
pub fn of(location: Location) -> Option<u16> {
    if location.slot().width != 2 {
        return None;
    }
    let h = match location {
        Location::Stack(s) => s.offset,
        Location::DirectPage(s) => 256 + s.offset,
    };
    valid(h).then_some(h)
}
pub fn valid(h: u16) -> bool {
    (1..=254).contains(&h) || ((288..=318).contains(&h) && h % 2 == 0)
}
pub fn load(v: (bool, u16)) -> Vec<u8> {
    if v.0 {
        memory(0xa3, 0xa5, v.1).to_vec()
    } else {
        vec![0xa9, v.1 as u8, (v.1 >> 8) as u8]
    }
}
pub fn store(h: u16) -> [u8; 2] {
    memory(0x83, 0x85, h)
}
fn memory(stack: u8, dp: u8, h: u16) -> [u8; 2] {
    assert!(valid(h));
    [if h < 256 { stack } else { dp }, h as u8]
}
pub fn address(s: u16, d: u16, h: u16) -> u32 {
    assert!(valid(h));
    if h < 256 {
        u32::from(s) + u32::from(h)
    } else {
        u32::from(d) + u32::from(h - 256)
    }
}
pub fn cycles(h: u16) -> u64 {
    assert!(valid(h));
    if h < 256 { 5 } else { 4 }
}
