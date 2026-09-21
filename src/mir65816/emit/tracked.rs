//! The only native instruction-writing boundary: each admitted form owns bytes
//! and effects together. Finalized Code remains patchable by the linker.
use super::{BlockId, Label, Slot, Target, TempId, code::Code, state::*};
use std::collections::{BTreeMap, BTreeSet};

macro_rules! instruction_set {
    ($name:ident { $($variant:ident = $byte:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(super) enum $name { $($variant),* }
        impl $name { fn opcode(self) -> u8 { match self { $(Self::$variant => $byte),* } } }
    };
}
instruction_set!(Implied { Clc=0x18, Sec=0x38, Tcs=0x1b, Tsc=0x3b, Tax=0xaa,
    Tay=0xa8, Tya=0x98, Txa=0x8a, Xba=0xeb, Phk=0x4b, Pha=0x48,
    DecA=0x3a, Rtl=0x6b, Dex=0xca, Nop=0xea });
instruction_set!(ByteOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    EorImm=0x49, LdaStack=0xa3, StaStack=0x83, AdcStack=0x63, SbcStack=0xe3,
    CmpStack=0xc3, LdaDp=0xa5, StaDp=0x85, LdxDp=0xa6, AdcDp=0x65,
    SbcDp=0xe5, CmpDp=0xc5, AndDp=0x25, OraDp=0x05, EorDp=0x45,
    AslDp=0x06, RolDp=0x26, LsrDp=0x46, RorDp=0x66,
    LdaIndirect=0xa7, StaIndirect=0x87, LdaIndirectY=0xb7, StaIndirectY=0x97,
    Rep=0xc2, Sep=0xe2 });
instruction_set!(WordOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    AndImm=0x29, LdyImm=0xa0 });
instruction_set!(LongOp { Lda=0xaf, Sta=0x8f });
instruction_set!(ReferenceOp { LdaLong=0xaf, StaLong=0x8f, LdaByte=0xa9, Jsl=0x22, Jml=0x5c });
instruction_set!(Branch { Plus=0x10, Minus=0x30, CarryClear=0x90, CarrySet=0xb0, NotEqual=0xd0, Equal=0xf0 });

#[derive(Clone, Copy, Debug)]
struct Entry {
    env: Environment,
    stack_a: Option<i32>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct TrackedEmitter65816 {
    code: Code,
    pub(super) state: State65816,
    entries: BTreeMap<Label, Entry>,
    bound: BTreeSet<Label>,
    blocks: BTreeSet<Label>,
    unreachable: bool,
    indirect_resume: Option<(Label, Environment)>,
    #[cfg(feature = "native65816-state-proof")]
    trace: Option<Vec<super::proof::Snapshot>>,
}
impl TrackedEmitter65816 {
    pub fn code(&self) -> &Code {
        &self.code
    }
    pub fn finish(self) -> Code {
        self.code
    }
    pub fn position(&self) -> usize {
        self.code.bytes.len()
    }
    pub fn span(&mut self, block: BlockId, index: usize, start: usize) {
        self.code
            .mir_spans
            .insert((block, index), start..self.position());
    }
    pub fn label(&mut self) -> Label {
        self.code.label()
    }
    pub fn declare_blocks(&mut self, labels: impl Iterator<Item = Label>) {
        assert_eq!(self.state.env.m, Width::Word);
        for label in labels {
            self.blocks.insert(label);
            self.entries.insert(
                label,
                Entry {
                    env: self.state.env,
                    stack_a: None,
                },
            );
        }
    }
    fn entry(&self) -> Entry {
        Entry {
            env: self.state.env,
            stack_a: match self.state.a {
                Value::StackAddress(s) => Some(s),
                _ => None,
            },
        }
    }
    fn edge(&mut self, label: Label) {
        let current = self.entry();
        if self.blocks.contains(&label) {
            assert_eq!(current.env.m, Width::Word, "MIR exit width");
        }
        if let Some(old) = self.entries.get_mut(&label) {
            assert_eq!(
                old.env, current.env,
                "incompatible execution contracts for {label:?}"
            );
            if old.stack_a != current.stack_a {
                assert!(
                    !self.bound.contains(&label) || old.stack_a.is_none(),
                    "backedge invalidates stack equation"
                );
                old.stack_a = None;
            }
        } else {
            self.entries.insert(label, current);
        }
    }
    pub fn mark(&mut self, label: Label) {
        if !self.unreachable {
            self.edge(label);
        }
        let entry = *self
            .entries
            .get(&label)
            .expect("label requires incoming execution contract");
        self.state.values_barrier();
        self.state.env = entry.env;
        if !self.blocks.contains(&label) {
            if let Some(s) = entry.stack_a {
                self.state.a = Value::StackAddress(s);
            }
        }
        self.state.mode_permission = false;
        self.code.mark(label);
        self.bound.insert(label);
        self.unreachable = false;
        self.observe();
    }
    pub fn word_cursor(&self) -> Option<(usize, usize)> {
        (self.state.env.m == Width::Word && self.state.mode_permission)
            .then_some((self.position(), self.code.labels.len()))
    }
    pub fn a8(&mut self) {
        self.accumulator_width(Width::Byte);
    }
    pub fn a16(&mut self) {
        self.accumulator_width(Width::Word);
    }
    fn accumulator_width(&mut self, width: Width) {
        if !self.state.mode_permission || self.state.env.m != width {
            self.byte(
                if width == Width::Byte {
                    ByteOp::Sep
                } else {
                    ByteOp::Rep
                },
                0x20,
            );
            self.state.mode_permission = true;
        }
    }
    pub fn delta(&self) -> u32 {
        self.state.delta()
    }
    pub fn establish_body(&mut self) {
        assert!(self.state.env.anchor.is_none() && self.state.env.pushes == 0);
        assert!(matches!(self.state.a,Value::StackAddress(s) if -s == self.state.env.depth));
        self.state.env.anchor = Some(self.state.env.depth);
    }
    pub fn barrier(&mut self) {
        self.state.adjacent = None;
    }
    pub fn register_home(&mut self, slot: Slot) {
        self.state.register_home(slot);
    }
    pub fn remember_word(&mut self, temp: TempId, slot: Slot) {
        if let Some(cursor) = self.word_cursor() {
            self.state.publish_word(temp, slot, cursor);
        }
    }
    pub fn consume_word(
        &mut self,
        temp: Option<TempId>,
        slot: Option<Slot>,
        offset: Option<u8>,
    ) -> bool {
        self.state
            .consume_word(temp, slot, offset, self.word_cursor())
    }
    fn live(&self) {
        assert!(
            !self.unreachable,
            "instruction without an execution contract"
        );
    }
    fn width(&self, expected: Width) {
        self.live();
        assert_eq!(self.state.env.m, expected, "immediate width");
    }
    pub fn op(&mut self, op: Implied) {
        use Implied::*;
        self.live();
        match op {
            Clc => self.state.carry = Some(false),
            Sec => self.state.carry = Some(true),
            Tsc => self
                .state
                .load_a(Value::StackAddress(-self.state.env.depth)),
            Tcs => {
                assert_eq!(self.state.env.pushes, 0);
                let Value::StackAddress(s) = self.state.a else {
                    panic!("TCS requires a checked stack equation");
                };
                self.state.env.depth = -s;
                self.state.peak = self.state.peak.max(-s);
                self.barrier();
            }
            Tax | Tay => {
                let value = self.state.narrow(self.state.a, self.state.env.index);
                if op == Tax {
                    self.state.x = value;
                } else {
                    self.state.y = value;
                }
                self.state.nz = value;
            }
            Txa | Tya => {
                let value = if op == Txa {
                    self.state.x
                } else {
                    self.state.y
                };
                let value = self.state.narrow(value, self.state.env.m);
                self.state.load_a(value);
            }
            Xba => {
                let value = match self.state.a {
                    Value::Constant(v, Width::Word) => {
                        Value::Constant(v.rotate_left(8), Width::Word)
                    }
                    _ => Value::Unknown,
                };
                let low = match value {
                    Value::Constant(v, _) => State65816::constant(v, Width::Byte),
                    _ => self.state.fresh(Width::Byte),
                };
                self.state.a = if self.state.env.m == Width::Byte {
                    low
                } else {
                    value
                };
                self.state.nz = low;
            }
            DecA | Dex => {
                let (value, width) = if op == Dex {
                    (self.state.x, self.state.env.index)
                } else {
                    (self.state.a, self.state.env.m)
                };
                let value = match value {
                    Value::Constant(v, _) => State65816::constant(v.wrapping_sub(1), width),
                    _ => self.state.fresh(width),
                };
                if op == Dex {
                    self.state.x = value;
                } else {
                    self.state.a = value;
                }
                self.state.nz = value;
            }
            Phk => self.state.push(1),
            Pha => self.state.push(self.state.env.m.bytes()),
            Rtl => {
                assert_eq!(self.state.env.m, Width::Word);
                assert_eq!(self.state.env.pushes, 0, "indirect RTL needs transfer API");
                assert_eq!(self.state.env.depth, 0, "return must restore entry S");
                self.unreachable = true;
            }
            Nop => {}
        }
        self.code.op(op.opcode());
        self.observe();
    }
    pub fn byte(&mut self, op: ByteOp, value: u8) {
        use ByteOp::*;
        self.live();
        let width = self.state.env.m;
        let immediate = matches!(op, LdaImm | AdcImm | SbcImm | CmpImm | EorImm);
        if immediate {
            self.width(Width::Byte);
        }
        let rhs = if immediate {
            State65816::constant(value.into(), width)
        } else if matches!(op, LdaStack | AdcStack | SbcStack | CmpStack) {
            self.state.read_stack(value, width)
        } else if op == LdxDp {
            self.state.fresh(self.state.env.index)
        } else {
            self.state.fresh(width)
        };
        match op {
            LdaImm | LdaStack | LdaDp | LdaIndirect | LdaIndirectY => self.state.load_a(rhs),
            StaStack => self.state.write_stack(value, width),
            StaDp | StaIndirect | StaIndirectY => self.state.unknown_write(),
            LdxDp => {
                self.state.x = rhs;
                self.state.nz = rhs;
            }
            AdcImm | AdcStack | AdcDp => self.state.arithmetic(rhs, false),
            SbcImm | SbcStack | SbcDp => self.state.arithmetic(rhs, true),
            CmpImm | CmpStack | CmpDp => self.state.compare(rhs),
            AndDp | OraDp | EorDp | EorImm => {
                let result = match (op, self.state.a, rhs) {
                    (EorImm, Value::Constant(a, w), Value::Constant(b, _)) => {
                        State65816::constant(a ^ b, w)
                    }
                    _ => self.state.fresh(width),
                };
                self.state.load_a(result);
            }
            AslDp | RolDp | LsrDp | RorDp => {
                self.state.unknown_write();
                self.state.nz = rhs;
                self.state.carry = None;
            }
            Rep | Sep => {
                self.state.status(value, op == Sep);
                self.state.mode_permission = false;
            }
        }
        self.code.byte(op.opcode(), value);
        self.observe();
    }
    pub fn word(&mut self, op: WordOp, value: u16) {
        use WordOp::*;
        self.live();
        if op == LdyImm {
            assert_eq!(self.state.env.index, Width::Word);
        } else {
            self.width(Width::Word);
        }
        let rhs = State65816::constant(value, Width::Word);
        match op {
            LdaImm => self.state.load_a(rhs),
            LdyImm => {
                self.state.y = rhs;
                self.state.nz = rhs;
            }
            AdcImm => self.state.arithmetic(rhs, false),
            SbcImm => self.state.arithmetic(rhs, true),
            CmpImm => self.state.compare(rhs),
            AndImm => {
                let result = match self.state.a {
                    Value::Constant(a, Width::Word) => State65816::constant(a & value, Width::Word),
                    _ => self.state.fresh(Width::Word),
                };
                self.state.load_a(result);
            }
        }
        self.code.word(op.opcode(), value);
        self.observe();
    }
    pub fn long(&mut self, op: LongOp, address: u32) -> Result<(), String> {
        if address >= 0x1000000 {
            return Err("24-bit instruction address overflow".into());
        }
        self.live();
        match op {
            LongOp::Lda => {
                let v = self.state.fresh(self.state.env.m);
                self.state.load_a(v);
            }
            LongOp::Sta => self.state.unknown_write(),
        }
        self.code.long(op.opcode(), address)?;
        self.observe();
        Ok(())
    }
    pub fn reference(&mut self, op: ReferenceOp, target: Target, addend: u32, byte: Option<u8>) {
        self.live();
        match op {
            ReferenceOp::LdaByte => {
                self.width(Width::Byte);
                assert!(byte.is_some());
                let v = self.state.fresh(Width::Byte);
                self.state.load_a(v);
            }
            ReferenceOp::LdaLong => {
                assert!(byte.is_none());
                let v = self.state.fresh(self.state.env.m);
                self.state.load_a(v);
            }
            ReferenceOp::StaLong => {
                assert!(byte.is_none());
                self.state.unknown_write();
            }
            ReferenceOp::Jsl => {
                assert!(
                    matches!(target, Target::Routine(_) | Target::Runtime(_)) && byte.is_none()
                );
                self.state.peak = self.state.peak.max(self.state.env.depth + 3);
                self.state.call_return();
            }
            ReferenceOp::Jml => {
                assert!(byte.is_none());
                if let Target::Label(label) = target {
                    self.edge(label);
                } else {
                    assert_eq!(target, Target::StackOverflow);
                }
                self.unreachable = true;
            }
        }
        self.code.reference(op.opcode(), target, addend, byte);
        self.observe();
    }
    pub fn jump(&mut self, label: Label) {
        self.reference(ReferenceOp::Jml, Target::Label(label), 0, None);
    }
    pub fn branch(&mut self, op: Branch, label: Label) {
        self.live();
        self.edge(label);
        // Inverse skip is an implicit continuation, not an unconditional exit.
        self.code.byte(op.opcode() ^ 0x20, 4);
        self.code.reference(0x5c, Target::Label(label), 0, None);
        self.observe();
    }
    pub fn push_return(&mut self, label: Label) {
        self.live();
        assert_eq!(self.state.env.pushes, 1);
        let mut continuation = self.state.env;
        continuation.depth -= 1;
        continuation.pushes = 0;
        self.indirect_resume = Some((label, continuation));
        self.state.push(2);
        self.code.push_return(label);
        self.observe();
    }
    pub fn indirect_transfer(&mut self) {
        self.live();
        assert_eq!(self.state.env.pushes, 6);
        assert_eq!(self.state.env.m, Width::Word);
        let (label, env) = self.indirect_resume.take().expect("PER continuation");
        self.code.op(0x6b);
        self.state.env.depth -= 3;
        self.state.env.pushes -= 3;
        self.observe(); // Callee entry: result/ABI postconditions do not apply yet.
        self.state.env = env;
        self.state.call_return();
        self.entries.insert(label, Entry { env, stack_a: None });
        self.unreachable = true;
    }
    fn observe(&mut self) {
        #[cfg(feature = "native65816-state-proof")]
        if let Some(trace) = self.trace.as_mut() {
            trace.push(super::proof::Snapshot::new(
                self.code.bytes.len(),
                &self.state,
            ));
        }
    }
    #[cfg(feature = "native65816-state-proof")]
    pub fn trace(&mut self) {
        self.trace = Some(Vec::new());
    }
    #[cfg(feature = "native65816-state-proof")]
    pub fn finish_traced(self) -> (Code, Vec<super::proof::Snapshot>) {
        (self.code, self.trace.unwrap_or_default())
    }
}
