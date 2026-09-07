//! Compiler-owned integer helpers, shared by classic and MIR6502 linking.
//! Independently implemented; no dependency on cartridge arithmetic bodies.
use crate::asm6502::{InlineAsmMode, InlineAsmRelocationKind, InlineAsmRelocationTarget, assemble};

pub(crate) mod wide;

/// Existing library convention for an invalid runtime argument (LIB.MSC Sound).
/// The cartridge's SysErr prints Y, not A. This is not a historical div/0 code.
pub(crate) const INVALID_ARGUMENT_ERROR: u8 = 100;
pub(crate) const CARTRIDGE_ERROR: u16 = 0x04CB;

pub(crate) struct IntegerHelperBody {
    pub(crate) bytes: Vec<u8>,
    pub(crate) error_operand: usize,
}

impl IntegerHelperBody {
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    #[cfg(test)]
    pub(crate) fn with_error_address(mut self, address: u16) -> Vec<u8> {
        self.bytes[self.error_operand..self.error_operand + 2]
            .copy_from_slice(&address.to_le_bytes());
        self.bytes
    }
}

fn fault_source() -> String {
    format!(
        "LDY #{INVALID_ARGUMENT_ERROR}\nTYA\nLDX #0\nJSR Error\n\
             CLD\nLDY #{INVALID_ARGUMENT_ERROR}\nTYA\nSEC\nfault: BCS fault\n"
    )
}

fn assemble_body(source: &str) -> IntegerHelperBody {
    let assembled = assemble(source, 0, InlineAsmMode::Analyzed)
        .expect("compiler-owned integer assembly must be valid");
    let symbolic = assembled
        .relocations
        .iter()
        .filter(|relocation| !matches!(relocation.target, InlineAsmRelocationTarget::Absolute(_)))
        .collect::<Vec<_>>();
    assert_eq!(symbolic.len(), 1, "only Error is relocatable");
    let relocation = symbolic[0];
    assert_eq!(relocation.kind, InlineAsmRelocationKind::Absolute16);
    assert_eq!(
        relocation.target,
        InlineAsmRelocationTarget::Symbol("Error".into())
    );
    IntegerHelperBody {
        bytes: assembled.bytes,
        error_operand: usize::from(relocation.offset),
    }
}

/// A:X dividend; $84:$85 divisor; A:X selected result. Clobbers A/X/Y/P,
/// $82..$87 and (signed only) $C2..$C3 on returning paths, with balanced stack.
/// Zero calls Error(100,0,100); if the handler returns, halt instead of resuming.
/// Error may have arbitrary memory/OS effects on the non-returning path.
pub(crate) fn division_body(signed: bool, remainder: bool) -> IntegerHelperBody {
    division_body_with_results(signed, remainder, false)
}

pub(crate) fn divmod_body(signed: bool) -> IntegerHelperBody {
    division_body_with_results(signed, false, true)
}

fn division_body_with_results(signed: bool, remainder: bool, both: bool) -> IntegerHelperBody {
    let mut source = String::from("CLD\nSTA $82\nSTX $83\nLDA $84\nORA $85\nBNE nonzero\n");
    source.push_str(&fault_source());
    source.push_str("nonzero:\n");
    if signed {
        source.push_str("STX $C2\nTXA\nEOR $85\nSTA $C3\nTXA\nBPL positive_left\n");
        source.push_str(&negate_word(0x82));
        source.push_str("positive_left: LDA $85\nBPL positive_right\n");
        source.push_str(&negate_word(0x84));
        source.push_str("positive_right:\n");
    }
    // Restoring unsigned division. The carry from ROL $87 is the seventeenth
    // remainder bit; it must force subtraction even if the low word is small.
    source.push_str(
        "LDA #0\nSTA $86\nSTA $87\nLDX #16\n\
         divide: ASL $82\nROL $83\nROL $86\nROL $87\nBCS subtract\n\
         LDA $87\nCMP $85\nBCC next\nBNE subtract\nLDA $86\nCMP $84\nBCC next\n\
         subtract: SEC\nLDA $86\nSBC $84\nSTA $86\nLDA $87\nSBC $85\nSTA $87\nINC $82\n\
         next: DEX\nBNE divide\n",
    );
    let result = if remainder { 0x86 } else { 0x82 };
    if both && signed {
        source.push_str("LDA $C2\nBPL remainder_ready\n");
        source.push_str(&negate_word(0x86));
        source.push_str("remainder_ready:\n");
    }
    if signed {
        source.push_str(if remainder { "LDA $C2\n" } else { "LDA $C3\n" });
        source.push_str("BPL result\n");
        source.push_str(&negate_word(result));
    }
    source.push_str(&format!(
        "result: LDA ${result:02X}\nLDX ${:02X}\n",
        result + 1
    ));
    // The existing routine emitter appends RTS.
    assemble_body(&source)
}

fn negate_word(low: u8) -> String {
    format!(
        "SEC\nLDA #0\nSBC ${low:02X}\nSTA ${low:02X}\n\
         LDA #0\nSBC ${high:02X}\nSTA ${high:02X}\n",
        high = low + 1,
    )
}

/// Unsigned / byte signatures. Byte/byte takes A,X; word/byte takes A:X,$84.
/// Quotient is word only for word/byte. Remainder is always unsigned byte.
pub(crate) fn narrow_division_body(word_dividend: bool, remainder: bool) -> IntegerHelperBody {
    let mut source = String::from("CLD\nSTA $82\n");
    source.push_str(if word_dividend {
        "STX $83\n"
    } else {
        "STX $84\n"
    });
    source.push_str("LDA $84\nBNE nonzero\n");
    source.push_str(&fault_source());
    source.push_str("nonzero: LDA #0\nSTA $86\n");
    source.push_str(if word_dividend {
        "LDX #16\n"
    } else {
        "LDX #8\n"
    });
    source.push_str("divide: ASL $82\n");
    if word_dividend {
        source.push_str("ROL $83\n");
    }
    source.push_str(
        "ROL $86\nBCS subtract\nLDA $86\nCMP $84\nBCC next\n\
        subtract: SEC\nLDA $86\nSBC $84\nSTA $86\nINC $82\nnext: DEX\nBNE divide\n",
    );
    source.push_str(if remainder { "LDA $86\n" } else { "LDA $82\n" });
    if word_dividend && !remainder {
        source.push_str("LDX $83\n");
    }
    assemble_body(&source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_bodies_only_relocate_the_error_call() {
        let mut bodies = Vec::new();
        for signed in [false, true] {
            bodies.push(divmod_body(signed));
            for remainder in [false, true] {
                bodies.push(division_body(signed, remainder));
                bodies.push(narrow_division_body(signed, remainder));
            }
        }
        assert_eq!(bodies.len(), 10);
        for body in bodies {
            assert_eq!(
                &body.bytes[body.error_operand - 6..body.error_operand],
                &[0xA0, 100, 0x98, 0xA2, 0, 0x20]
            ); // LDY #100; TYA; LDX #0; JSR
            assert_eq!(
                &body.bytes[body.error_operand + 2..body.error_operand + 9],
                &[0xD8, 0xA0, 100, 0x98, 0x38, 0xB0, 0xFE]
            ); // defensive non-return guard
        }
    }
}
