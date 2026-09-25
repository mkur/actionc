//! Zero-frame native word cores. All state belongs to the current D page:
//! $00..03 dividend/multiplicand, $04..07 divisor/multiplier, $08..0b product,
//! $0c..0f remainder, $10..11 quotient sign, $12..13 remainder sign.
//! X is the bounded loop counter. No scratch survives a call; no nested calls.
use super::*;
use crate::mir65816::arithmetic::{Helper, Operation};
const LEFT: u8 = 0;
const RHS: u8 = 4;
const PRODUCT: u8 = 8;
const REM: u8 = 12;
const QSIGN: u8 = 16;
const RSIGN: u8 = 18;

pub(super) fn emit(
    routine: &Mir65816Routine,
    helper: Helper,
    trace: bool,
) -> Result<MachineRoutine, String> {
    let frame = AllocatedFrame {
        extent: 0,
        spill_bytes: 0,
        peak_below_entry: 0,
        temps: BTreeMap::new(),
        edge_copies: vec![],
    };
    let mut b = Builder {
        routine,
        frame,
        code: TrackedEmitter65816::for_entry(routine.prologue.required_mode),
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    };
    #[cfg(feature = "native65816-state-proof")]
    if trace {
        b.code.trace();
    }
    let _ = trace;
    b.check_stack(0);
    b.code.op(Implied::Tcs);
    b.code.establish_body();
    let words = if helper.bytes <= 2 { 1 } else { 2 };
    for (i, target) in [LEFT, RHS].into_iter().enumerate() {
        let source = b.incoming(ParamId(i as u32))? as u8;
        for w in 0..words {
            let remaining = helper.bytes.saturating_sub(w * 2);
            if remaining >= 2 {
                b.code.byte(ByteOp::LdaStack, source + w * 2);
            } else {
                b.code.word(WordOp::LdaImm, 0);
                b.code.a8();
                b.code.byte(ByteOp::LdaStack, source + w * 2);
                b.code.a16();
            }
            b.code.byte(ByteOp::StaDp, target + w * 2);
        }
    }
    if helper.operation == Operation::Multiply {
        b.multiply(words);
        b.helper_result(PRODUCT, helper.bytes)?;
    } else {
        let nonzero = b.code.label();
        b.code.byte(ByteOp::LdaDp, RHS);
        if words == 2 {
            b.code.byte(ByteOp::OraDp, RHS + 2);
        }
        b.code.dispatch(Branch::NotEqual, nonzero);
        b.arithmetic_fault();
        b.code.mark(nonzero);
        if helper.signed {
            b.code.byte(ByteOp::LdaDp, LEFT + (words - 1) * 2);
            b.code.byte(ByteOp::StaDp, RSIGN);
            b.code.byte(ByteOp::EorDp, RHS + (words - 1) * 2);
            b.code.byte(ByteOp::StaDp, QSIGN);
            b.magnitude(LEFT, words);
            b.magnitude(RHS, words);
        }
        b.divide(words);
        let (result, sign) = if helper.operation == Operation::Divide {
            (LEFT, QSIGN)
        } else {
            (REM, RSIGN)
        };
        if helper.signed {
            let positive = b.code.label();
            b.code.byte(ByteOp::LdaDp, sign);
            b.code.dispatch(Branch::Plus, positive);
            b.negate(result, words);
            b.code.mark(positive);
        }
        b.helper_result(result, helper.bytes)?;
    }
    let code = b.code.finish_selected(routine.id, &b.frame, None)?;
    // Replayed with the same physical-effects and selected-CFG checks as user code.
    let code = super::super::rewrite::pilot::apply(&code, &[], trace)?;
    Ok(MachineRoutine {
        id: routine.id,
        frame: b.frame,
        code,
    })
}
impl Builder<'_> {
    pub(super) fn arithmetic_fault(&mut self) {
        self.code.a16();
        self.code.op(Implied::Tsc);
        self.code.op(Implied::Tax);
        self.code.word(WordOp::LdaImm, 1); // Native fault ABI: DivisionByZero.
        self.code
            .reference(ReferenceOp::Jml, Target::ArithmeticFault, 0, None);
    }
    fn zero(&mut self, target: u8, words: u8) {
        self.code.word(WordOp::LdaImm, 0);
        for w in 0..words {
            self.code.byte(ByteOp::StaDp, target + 2 * w);
        }
    }
    fn shift_left(&mut self, target: u8, words: u8) {
        self.code.byte(ByteOp::AslDp, target);
        if words == 2 {
            self.code.byte(ByteOp::RolDp, target + 2);
        }
    }
    fn multiply(&mut self, words: u8) {
        self.zero(PRODUCT, words);
        self.code.word(WordOp::LdaImm, u16::from(words) * 16);
        self.code.op(Implied::Tax);
        let again = self.code.label();
        let skip = self.code.label();
        self.code.mark(again);
        self.code.byte(ByteOp::LsrDp, RHS + (words - 1) * 2);
        if words == 2 {
            self.code.byte(ByteOp::RorDp, RHS);
        }
        self.code.dispatch(Branch::CarryClear, skip);
        self.code.op(Implied::Clc);
        for w in 0..words {
            self.code.byte(ByteOp::LdaDp, PRODUCT + w * 2);
            self.code.byte(ByteOp::AdcDp, LEFT + w * 2);
            self.code.byte(ByteOp::StaDp, PRODUCT + w * 2);
        }
        self.code.mark(skip);
        self.shift_left(LEFT, words);
        self.code.op(Implied::Dex);
        self.code.dispatch(Branch::NotEqual, again);
    }
    fn negate(&mut self, target: u8, words: u8) {
        self.code.op(Implied::Sec);
        for w in 0..words {
            self.code.word(WordOp::LdaImm, 0);
            self.code.byte(ByteOp::SbcDp, target + w * 2);
            self.code.byte(ByteOp::StaDp, target + w * 2);
        }
    }
    fn magnitude(&mut self, target: u8, words: u8) {
        let positive = self.code.label();
        self.code.byte(ByteOp::LdaDp, target + (words - 1) * 2);
        self.code.dispatch(Branch::Plus, positive);
        self.negate(target, words);
        self.code.mark(positive);
    }
    fn divide(&mut self, words: u8) {
        self.zero(REM, words);
        self.code.word(WordOp::LdaImm, u16::from(words) * 16);
        self.code.op(Implied::Tax);
        let again = self.code.label();
        let subtract = self.code.label();
        let skip = self.code.label();
        self.code.mark(again);
        self.shift_left(LEFT, words); // Quotient accumulates in the consumed dividend.
        for w in 0..words {
            self.code.byte(ByteOp::RolDp, REM + w * 2);
        }
        // Keep the extra remainder bit before CMP overwrites carry.
        self.code.dispatch(Branch::CarrySet, subtract);
        for w in (0..words).rev() {
            self.code.byte(ByteOp::LdaDp, REM + w * 2);
            self.code.byte(ByteOp::CmpDp, RHS + w * 2);
            self.code.dispatch(Branch::CarryClear, skip);
            if w != 0 {
                self.code.dispatch(Branch::NotEqual, subtract);
            }
        }
        self.code.mark(subtract);
        self.code.op(Implied::Sec);
        for w in 0..words {
            self.code.byte(ByteOp::LdaDp, REM + w * 2);
            self.code.byte(ByteOp::SbcDp, RHS + w * 2);
            self.code.byte(ByteOp::StaDp, REM + w * 2);
        }
        self.code.byte(ByteOp::LdaDp, LEFT);
        self.code.word(WordOp::EorImm, 1); // ASL cleared this quotient bit.
        self.code.byte(ByteOp::StaDp, LEFT);
        self.code.mark(skip);
        self.code.op(Implied::Dex);
        self.code.dispatch(Branch::NotEqual, again);
    }
    fn helper_result(&mut self, result: u8, bytes: u8) -> Result<(), String> {
        if bytes > 2 {
            self.code.byte(ByteOp::LdaDp, result + 2);
            if bytes == 3 {
                self.code.word(WordOp::AndImm, 0xff);
            }
            self.code.op(Implied::Tax);
        }
        self.code.byte(ByteOp::LdaDp, result);
        if bytes == 1 {
            self.code.word(WordOp::AndImm, 0xff);
        }
        self.code.native_return(self.routine.result_home)
    }
}
