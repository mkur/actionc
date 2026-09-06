//! Compiler-owned integer helpers, shared by classic and MIR6502 linking.
//! Independently implemented; no dependency on cartridge arithmetic bodies.
use crate::asm6502::{InlineAsmMode, assemble};

/// A:X dividend; $84:$85 divisor; A:X selected result. Clobbers A/X/Y/P,
/// $82..$87 and (signed only) $C2..$C3. No absolute memory writes, balanced
/// stack on returning paths. Zero never returns: A=1, carry set, BCS self.
pub(crate) fn division_body(signed: bool, remainder: bool) -> Vec<u8> {
    division_body_with_results(signed, remainder, false)
}

pub(crate) fn divmod_body(signed: bool) -> Vec<u8> {
    division_body_with_results(signed, false, true)
}

fn division_body_with_results(signed: bool, remainder: bool, both: bool) -> Vec<u8> {
    let mut source = String::from(
        "CLD\nSTA $82\nSTX $83\nLDA $84\nORA $85\nBNE nonzero\n\
         LDA #1\nSEC\nfault: BCS fault\nnonzero:\n",
    );
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
    let assembled = assemble(&source, 0, InlineAsmMode::Analyzed)
        .expect("compiler-owned division assembly must be valid");
    assert!(
        assembled.relocations.iter().all(|relocation| matches!(
            relocation.target,
            crate::asm6502::InlineAsmRelocationTarget::Absolute(_)
        )),
        "helper uses only relative control flow: {:?}",
        assembled.relocations
    );
    // The existing routine emitter appends RTS.
    assembled.bytes
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
pub(crate) fn narrow_division_body(word_dividend: bool, remainder: bool) -> Vec<u8> {
    let mut source = String::from("CLD\nSTA $82\n");
    source.push_str(if word_dividend {
        "STX $83\n"
    } else {
        "STX $84\n"
    });
    source.push_str(
        "LDA $84\nBNE nonzero\nLDA #1\nSEC\nfault: BCS fault\nnonzero: LDA #0\nSTA $86\n",
    );
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
    let assembled =
        assemble(&source, 0, InlineAsmMode::Analyzed).expect("valid narrow division helper");
    assert!(assembled.relocations.iter().all(|relocation| matches!(
        relocation.target,
        crate::asm6502::InlineAsmRelocationTarget::Absolute(_)
    )));
    assembled.bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_bodies_are_position_independent() {
        for signed in [false, true] {
            for remainder in [false, true] {
                assert!(!division_body(signed, remainder).is_empty());
            }
        }
    }
}
