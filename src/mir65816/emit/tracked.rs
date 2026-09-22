//! The only native instruction-writing boundary: each admitted form owns bytes
//! and effects together. Finalized Code remains patchable by the linker.
use super::{BlockId, Location, Slot, TempId, copies::WordHome, state::*};
#[path = "code.rs"]
mod encoding;
pub use encoding::{Code, ConditionalBranch, Fixup, Label, MirTransfer, Target};
use std::collections::{BTreeMap, BTreeSet};
#[path = "x_state.rs"]
mod x_state;
pub(super) use x_state::XContract;

use super::effects::CallContract;
pub(super) use super::selected::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Instruction,
    Join,
    CallReturn,
    IndirectTransfer,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    env: Environment,
    stack_a: Option<i64>,
    x_word: bool,
}
#[derive(Clone, Debug, Default)]
pub(super) struct TrackedEmitter65816 {
    code: Code,
    state: State65816,
    x_contract: Option<XContract>,
    x_reserved: bool,
    x_valid: bool,
    x_refreshed: bool,
    x_access: bool,
    entries: BTreeMap<Label, Entry>,
    bound: BTreeSet<Label>,
    blocks: BTreeSet<Label>,
    proved_blocks: BTreeSet<Label>,
    remaining_edges: Option<BTreeMap<Label, BTreeMap<Option<Label>, usize>>>,
    active_block: Option<Label>,
    pending_fallthrough: Option<Label>,
    unreachable: bool,
    indirect_resume: Option<(Label, Environment)>,
    #[cfg(feature = "native65816-state-proof")]
    trace: Option<Vec<super::proof::Snapshot>>,
}
impl TrackedEmitter65816 {
    pub fn for_entry(mode: super::super::Mir65816ModeState) -> Self {
        assert!(mode.native_mode);
        assert_eq!(
            mode.accumulator,
            super::super::Mir65816RegisterWidth::Bits16
        );
        assert_eq!(mode.index, super::super::Mir65816RegisterWidth::Bits16);
        Self::default()
    }

    #[cfg(test)]
    pub fn for_test(frame: &super::AllocatedFrame) -> Self {
        let mut e = Self::default();
        e.test_frame(frame.extent);
        for home in frame.temps.values() {
            if let super::Location::Stack(slot) = home {
                e.register_home(*slot);
            }
        }
        e
    }
    #[cfg(test)]
    pub fn test_delta(&mut self, delta: u32) {
        self.state.env.depth = self.state.env.anchor.unwrap_or(0) + i64::from(delta);
    }

    #[cfg(test)]
    pub fn test_frame(&mut self, bytes: u16) {
        self.state.env.depth = i64::from(bytes);
        self.state.env.anchor = Some(i64::from(bytes));
    }

    pub fn code(&self) -> &Code {
        &self.code
    }
    pub fn finish(self) -> Code {
        assert!(
            self.pending_fallthrough.is_none(),
            "unbound fallthrough target"
        );
        if let Some(edges) = &self.remaining_edges {
            assert!(
                edges.values().all(|p| p.values().all(|&n| n == 0)),
                "unchecked MIR predecessors"
            );
            assert!(
                edges.keys().all(|l| self.bound.contains(l)),
                "unbound MIR block"
            );
        }
        #[cfg(feature = "native65816-state-proof")]
        {
            let mut code = self.code;
            code.state_trace = self.trace.unwrap_or_default();
            code
        }
        #[cfg(not(feature = "native65816-state-proof"))]
        {
            self.code
        }
    }
    #[cfg(test)]
    pub fn state_for_incoming_test(&self) -> State65816 {
        self.state.clone()
    }

    #[cfg(test)]
    pub fn resident(&self) -> Option<AdjacentWord> {
        self.state.adjacent
    }
    #[cfg(test)]
    pub fn peak(&self) -> i64 {
        self.state.peak
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
                    x_word: false,
                },
            );
        }
    }
    /// Expected CFG transfers are obligations, not observations. Every actual
    /// exit is checked, including backedges emitted after an eligible binding.
    pub fn prove_entries(
        &mut self,
        predecessors: BTreeMap<Label, BTreeMap<Option<Label>, usize>>,
        reachable: BTreeSet<Label>,
    ) {
        let e = self.state.env;
        assert!(e.native && e.m == Width::Word && e.index == Width::Word);
        assert!(
            e.anchor == Some(e.depth) && e.pushes == 0,
            "MIR body stack contract"
        );
        assert_eq!(
            predecessors.keys().copied().collect::<BTreeSet<_>>(),
            self.blocks
        );
        assert!(reachable.is_subset(&self.blocks));
        assert!(
            reachable
                .iter()
                .all(|l| predecessors[l].values().any(|&n| n > 0))
        );
        assert!(self.remaining_edges.is_none() && self.active_block.is_none());
        self.remaining_edges = Some(predecessors);
        self.proved_blocks = reachable;
    }
    fn entry(&self) -> Entry {
        Entry {
            env: self.state.env,
            x_word: self.x_valid,
            stack_a: match self.state.a {
                Value::StackAddress(s) => Some(s),
                _ => None,
            },
        }
    }
    fn edge(&mut self, label: Label) {
        let mut current = self.entry();
        current.x_word = self.x_edge(label);
        if self.blocks.contains(&label) {
            assert_eq!(current.env.m, Width::Word, "MIR exit width");
            if let Some(edges) = &mut self.remaining_edges {
                let count = edges
                    .get_mut(&label)
                    .and_then(|p| p.get_mut(&self.active_block))
                    .expect("unexpected MIR predecessor");
                *count = count.checked_sub(1).expect("duplicate MIR predecessor");
            }
        }
        if let Some(old) = self.entries.get_mut(&label) {
            assert_eq!(old.x_word, current.x_word, "incompatible X entry");
            old.env.irq_preserved &= current.env.irq_preserved;
            let mut incoming = current.env;
            incoming.irq_preserved = old.env.irq_preserved;
            assert_eq!(
                old.env, incoming,
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
        if let Some(target) = self.pending_fallthrough.take() {
            assert_eq!(target, label, "fallthrough must bind the next MIR block");
        }
        if !self.unreachable {
            self.edge(label);
        }
        let entry = *self
            .entries
            .get(&label)
            .expect("label requires incoming execution contract");
        self.state.values_barrier();
        self.state.env = entry.env;
        self.x_join(entry.x_word);
        self.state.env.irq_preserved = false;
        if !self.blocks.contains(&label) {
            if let Some(s) = entry.stack_a {
                self.state.a = Value::StackAddress(s);
            }
        }
        self.state.mode_permission = self.proved_blocks.contains(&label);
        if self.blocks.contains(&label) {
            self.active_block = Some(label);
        }
        self.code.mark(label);
        self.bound.insert(label);
        self.unreachable = false;
        self.observe_event(Event::Join);
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
        self.state.incoming = None;
    }
    pub fn register_home(&mut self, slot: impl Into<Location>) {
        self.state.register_home(slot);
    }
    pub fn remember_word(&mut self, temp: TempId, slot: impl Into<Location>) {
        if let Some(cursor) = self.word_cursor() {
            self.state.publish_word(temp, slot, cursor);
        }
    }
    pub fn consume_word(
        &mut self,
        temp: Option<TempId>,
        slot: Option<Location>,
        offset: Option<WordHome>,
    ) -> bool {
        self.state
            .consume_word(temp, slot, offset, self.word_cursor())
    }
    pub fn remember_frame_word(
        &mut self,
        object: super::Mir65816FrameObjectId,
        byte: u32,
        slot: Slot,
    ) {
        if let Some(cursor) = self.word_cursor() {
            self.state
                .publish_adjacent(WordIdentity::Frame(object, byte), slot, cursor);
        }
    }
    pub fn consume_frame_word(
        &mut self,
        object: super::Mir65816FrameObjectId,
        byte: u32,
        slot: Slot,
        offset: u8,
    ) -> bool {
        self.state.consume_adjacent(
            Some(WordIdentity::Frame(object, byte)),
            Some(Location::Stack(slot)),
            Some(WordHome::Stack(offset)),
            self.word_cursor(),
        )
    }
    /// Both homes have already passed target extent/alias checks. A real read
    /// establishes provenance; an omitted consumer never rearms this witness.
    pub fn capture_incoming_word(
        &mut self,
        param: super::ParamId,
        source: Slot,
        temp: TempId,
        destination: Location,
    ) {
        let forwarded = self
            .state
            .consume_incoming(param, source, self.word_cursor());
        self.barrier();
        self.a16();
        if !forwarded {
            self.byte(ByteOp::LdaStack, source.offset as u8);
            self.state.record_incoming_read(source);
            self.observe();
        }
        self.store_word(super::copies::word_home(destination, 0).expect("preflighted capture"));
        self.remember_word(temp, destination);
        if !forwarded {
            self.state
                .publish_incoming(param, source, self.word_cursor().unwrap());
        }
    }
    /// The sole admitted extension: consume the original capture without LDA,
    /// emit exactly one disjoint STA16, then grant one final parameter consumer.
    pub fn store_incoming_capture(
        &mut self,
        temp: TempId,
        source: Location,
        destination: Slot,
    ) -> bool {
        let Some(mut fact) = self.state.incoming.take() else {
            return false;
        };
        let disjoint = |a: Slot, b: Slot| {
            u32::from(a.offset) + u32::from(a.width) <= u32::from(b.offset)
                || u32::from(b.offset) + u32::from(b.width) <= u32::from(a.offset)
        };
        if fact.stored
            || fact.capture.identity != WordIdentity::Temp(temp)
            || fact.capture.slot != source
            || destination.width != 2
            || !disjoint(destination, fact.source)
            || Location::Stack(destination).overlaps(source)
            || !self.state.incoming_matches(fact, self.word_cursor())
            || !self.consume_word(
                Some(temp),
                Some(source),
                Some(super::copies::word_home(source, 0).expect("preflighted source")),
            )
        {
            return false;
        }
        self.byte(ByteOp::StaStack, destination.offset as u8);
        self.barrier();
        fact.cursor.0 += 2;
        fact.stored = true;
        assert!(self.state.incoming_matches(fact, self.word_cursor()));
        self.state.incoming = Some(fact);
        true
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
    // The only instruction dispatch. Encoding and forward-state helpers cannot
    // be called by selectors without deriving the corresponding typed effects.
    fn instruction(&mut self, instruction: Instruction) -> Result<(), String> {
        let effects = instruction.effects(self.state.env);
        #[cfg(feature = "native65816-state-proof")]
        let start = self.position();
        match instruction {
            Instruction::Implied(op) => self.emit_implied(op),
            Instruction::Byte(op, value) => self.emit_byte(op, value),
            Instruction::Word(op, value) => self.emit_word(op, value),
            Instruction::Long(op, value) => self.emit_long(op, value)?,
            Instruction::Reference(op, target, addend, byte) => {
                self.emit_reference(op, target, addend, byte)
            }
            Instruction::Branch(op, label) => self.emit_branch(op, label),
            Instruction::PushReturn(label) => self.emit_push_return(label),
            Instruction::IndirectTransfer(_) => self.emit_indirect_transfer(),
            Instruction::NativeCall(target, _) => {
                self.emit_reference(ReferenceOp::Jsl, target, 0, None)
            }
            Instruction::NativeReturn(_) => self.emit_implied(Implied::Rtl),
        }
        #[cfg(feature = "native65816-state-proof")]
        if self.trace.is_some() {
            self.code
                .instruction_effects
                .push(super::effects::EffectRecord {
                    start,
                    end: self.position(),
                    effects,
                });
            return Ok(());
        }
        let _ = effects;
        Ok(())
    }
    pub fn op(&mut self, op: Implied) {
        self.instruction(Instruction::Implied(op))
            .expect("infallible implied instruction");
    }
    pub fn byte(&mut self, op: ByteOp, value: u8) {
        self.instruction(Instruction::Byte(op, value))
            .expect("infallible byte operand");
    }
    pub fn word(&mut self, op: WordOp, value: u16) {
        self.instruction(Instruction::Word(op, value))
            .expect("infallible word operand");
    }
    pub fn long(&mut self, op: LongOp, value: u32) -> Result<(), String> {
        self.instruction(Instruction::Long(op, value))
    }
    pub fn reference(&mut self, op: ReferenceOp, target: Target, addend: u32, byte: Option<u8>) {
        self.instruction(Instruction::Reference(op, target, addend, byte))
            .expect("infallible reference");
    }
    pub fn branch(&mut self, op: Branch, label: Label) {
        self.instruction(Instruction::Branch(op, label))
            .expect("infallible branch");
    }
    pub fn push_return(&mut self, label: Label) {
        self.instruction(Instruction::PushReturn(label))
            .expect("infallible PER");
    }
    #[cfg(test)]
    pub fn indirect_transfer(&mut self) {
        self.instruction(Instruction::IndirectTransfer(None))
            .expect("infallible indirect transfer");
    }
    pub fn native_call(
        &mut self,
        target: Target,
        plan: &super::Mir65816CallPlan,
    ) -> Result<(), String> {
        let contract = CallContract::from_plan(plan, super::super::abi::FarTransfer::Jsl)?;
        self.instruction(Instruction::NativeCall(target, contract))
    }
    pub fn native_indirect_transfer(
        &mut self,
        plan: &super::Mir65816CallPlan,
    ) -> Result<(), String> {
        let contract = CallContract::from_plan(plan, super::super::abi::FarTransfer::StackRtl)?;
        self.instruction(Instruction::IndirectTransfer(Some(contract)))
    }
    pub fn native_return(&mut self, home: Option<super::Mir65816AbiHome>) -> Result<(), String> {
        self.instruction(Instruction::NativeReturn(CallContract::result(home)?))
    }
    fn emit_implied(&mut self, op: Implied) {
        use Implied::*;
        self.live();
        self.x_implied(op);
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
                self.state.homes.clear();
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
            DecA | Dex | Inx => {
                let (value, width) = if matches!(op, Dex | Inx) {
                    (self.state.x, self.state.env.index)
                } else {
                    (self.state.a, self.state.env.m)
                };
                let value = match value {
                    Value::Constant(v, _) => State65816::constant(
                        if op == Inx {
                            v.wrapping_add(1)
                        } else {
                            v.wrapping_sub(1)
                        },
                        width,
                    ),
                    _ => self.state.fresh(width),
                };
                if matches!(op, Dex | Inx) {
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
    pub fn store_word(&mut self, home: WordHome) {
        match home {
            WordHome::Stack(offset) => self.byte(ByteOp::StaStack, offset),
            WordHome::DirectPage(offset) => self.byte(ByteOp::StaDp, offset),
        }
    }
    fn emit_byte(&mut self, op: ByteOp, value: u8) {
        use ByteOp::*;
        self.live();
        self.x_byte(op, value);
        let width = self.state.env.m;
        let immediate = matches!(op, LdaImm | AdcImm | SbcImm | CmpImm | EorImm);
        if immediate {
            self.width(Width::Byte);
        }
        let rhs = if immediate {
            State65816::constant(value.into(), width)
        } else if matches!(op, LdaStack | AdcStack | SbcStack | CmpStack) {
            self.state.read_stack(value, width)
        } else if matches!(op, LdaDp | AdcDp | SbcDp | CmpDp) {
            self.state.read_dp(value, width)
        } else if op == LdxDp {
            self.state.fresh(self.state.env.index)
        } else {
            self.state.fresh(width)
        };
        match op {
            LdaImm | LdaStack | LdaDp | LdaIndirect | LdaIndirectY => self.state.load_a(rhs),
            StaStack => self.state.write_stack(value, width),
            StaDp => self.state.write_dp(value, width),
            StaIndirect | StaIndirectY => self.state.unknown_write(),
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
    fn emit_word(&mut self, op: WordOp, value: u16) {
        use WordOp::*;
        self.live();
        if self.x_reserved {
            assert!(op != CpxImm || self.x_access, "unplanned X compare");
            assert!(!matches!(op, LdyImm), "instruction clobbers X reservation");
        }
        if matches!(op, LdyImm | CpxImm) {
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
            CpxImm => self
                .state
                .compare_value(self.state.x, rhs, self.state.env.index),
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
    fn emit_long(&mut self, op: LongOp, address: u32) -> Result<(), String> {
        assert!(!self.x_reserved, "unmodelled X-region memory access");
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
    fn emit_reference(&mut self, op: ReferenceOp, target: Target, addend: u32, byte: Option<u8>) {
        assert!(
            !self.x_reserved || matches!((&op, &target), (ReferenceOp::Jml, Target::Label(_))),
            "X-region call or reference"
        );
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
        self.observe_event(if op == ReferenceOp::Jsl {
            Event::CallReturn
        } else {
            Event::Instruction
        });
    }
    fn record_transfer(&mut self, label: Label, fallthrough: bool) {
        if self.blocks.contains(&label) {
            self.code.mir_transfers.push(MirTransfer {
                source: self.active_block.expect("MIR transfer source"),
                target: label,
                offset: self.position(),
                fallthrough,
            });
        }
    }
    pub fn jump(&mut self, label: Label) {
        self.record_transfer(label, false);
        self.reference(ReferenceOp::Jml, Target::Label(label), 0, None);
    }
    pub fn fallthrough(&mut self, label: Label) {
        self.live();
        assert!(
            self.blocks.contains(&label),
            "fallthrough requires a MIR block"
        );
        self.edge(label);
        self.record_transfer(label, true);
        self.unreachable = true;
        self.pending_fallthrough = Some(label);
    }
    fn emit_branch(&mut self, op: Branch, label: Label) {
        self.live();
        self.edge(label);
        // Inverse skip is an implicit continuation, not an unconditional exit.
        self.code.byte(op.opcode() ^ 0x20, 4);
        self.code.reference(0x5c, Target::Label(label), 0, None);
        self.observe();
    }
    pub fn dispatch(&mut self, op: Branch, label: Label) {
        let offset = self.position();
        self.branch(op, label);
        self.code.conditional_branches.push(ConditionalBranch {
            offset,
            predicate: op.opcode(),
            target: label,
            short: false,
        });
    }
    fn emit_push_return(&mut self, label: Label) {
        self.live();
        assert!(!self.x_reserved, "X-region indirect call");
        assert_eq!(self.state.env.pushes, 1);
        let mut continuation = self.state.env;
        continuation.depth -= 1;
        continuation.pushes = 0;
        self.indirect_resume = Some((label, continuation));
        self.state.push(2);
        self.code.push_return(label);
        self.observe();
    }
    fn emit_indirect_transfer(&mut self) {
        self.live();
        assert_eq!(self.state.env.pushes, 6);
        assert_eq!(self.state.env.m, Width::Word);
        let (label, env) = self.indirect_resume.take().expect("PER continuation");
        self.code.op(0x6b);
        self.state.env.depth -= 3;
        self.state.env.pushes -= 3;
        self.observe_event(Event::IndirectTransfer); // Callee entry: result/ABI postconditions do not apply yet.
        self.state.env = env;
        self.state.call_return();
        self.entries.insert(
            label,
            Entry {
                env,
                stack_a: None,
                x_word: false,
            },
        );
        self.unreachable = true;
    }
    fn observe(&mut self) {
        self.observe_event(Event::Instruction);
    }
    fn observe_event(&mut self, _event: Event) {
        self.code.boundaries.insert(self.position());
        #[cfg(feature = "native65816-state-proof")]
        if let Some(trace) = self.trace.as_mut() {
            let mut snapshot = super::proof::Snapshot::new(self.code.bytes.len(), &self.state);
            snapshot.event = _event;
            trace.push(snapshot);
        }
    }
    #[cfg(feature = "native65816-state-proof")]
    pub fn trace(&mut self) {
        self.trace = Some(vec![super::proof::Snapshot::new(0, &self.state)]);
    }
    #[cfg(feature = "native65816-state-proof")]
    pub fn finish_traced(self) -> (Code, Vec<super::proof::Snapshot>) {
        (self.code, self.trace.unwrap_or_default())
    }
}
