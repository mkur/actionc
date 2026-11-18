//! Implementation identity for reviewed 6502 leaf contracts. A successful
//! match certifies the recorded claims, independently of instruction effects.
use crate::codegen::{AddressingMode, decode_6502_opcode};

pub(crate) enum BodyItem<'a> {
    Byte(u8),
    Global { name: &'a str, extent: u16 },
}

pub(crate) fn fingerprint(params: &[u8], result: u8, items: &[BodyItem<'_>], version: &[u8]) -> Option<u64> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    };
    add(version);
    add(params);
    add(&[0, result]);
    let mut iter = items.iter().peekable();
    while let Some(BodyItem::Byte(opcode)) = iter.next() {
        let (op, mode, len) = decode_6502_opcode(*opcode)?;
        add(&[0, *opcode]);
        if op == "RTS" {
            return iter.peek().is_none().then_some(hash);
        }
        if mode == AddressingMode::Relative || matches!(op,
            "JSR" | "JMP" | "BRK" | "RTI" | "PHA" | "PLA" | "PHP" | "PLP" | "TXS" | "TSX") {
            return None;
        }
        let mut left = len.checked_sub(1)?;
        while left > 0 {
            match iter.next()? {
                BodyItem::Byte(b) => { add(&[0, *b]); left -= 1; }
                BodyItem::Global { name, extent } if left == 2 => {
                    add(&[1]);
                    add(&extent.to_le_bytes());
                    add(&u32::try_from(name.len()).ok()?.to_le_bytes());
                    add(name.as_bytes());
                    left -= 2;
                }
                _ => return None,
            }
        }
    }
    None
}

pub(crate) const ENTRY_VERSION: &[u8] = b"actionc-6502-abi-inputs-v1";
