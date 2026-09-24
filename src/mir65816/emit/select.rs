use super::{
    allocation::width,
    copies::{self, WordOperand, word_home},
    *,
};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
#[path = "word_tests.rs"]
pub(super) mod word_tests;

#[cfg(test)]
#[path = "compare_tests.rs"]
mod compare_tests;

#[cfg(test)]
#[path = "narrow_compare_tests.rs"]
mod narrow_compare_tests;

#[cfg(test)]
#[path = "branch_tests.rs"]
mod branch_tests;

#[cfg(test)]
#[path = "edge_tests.rs"]
mod edge_tests;
#[cfg(test)]
#[path = "staging_tests.rs"]
mod staging_tests;
#[cfg(test)]
use copies::acyclic_word_order;

#[path = "accumulator.rs"]
mod accumulator;
#[path = "arithmetic.rs"]
mod arithmetic;
#[path = "parameter.rs"]
mod parameter;
use super::tracked::*;

#[cfg(test)]
#[path = "accumulator_tests.rs"]
mod accumulator_tests;

#[cfg(test)]
#[path = "scalar_word_tests.rs"]
mod scalar_word_tests;

#[cfg(test)]
#[path = "call_tests.rs"]
mod call_tests;

// ABI call-clobbered domain scratch. Nothing here survives a call.
const PTR: u8 = abi::generated::DP_POINTER0_OFFSET as u8;
const RESULT: u8 = 8;
const RIGHT: u8 = 16;
const INDEX: u8 = 20;

/// The verified ABI homes define payload; their intervening and trailing gaps
/// still need explicit zero stores. Preflight before emitting a call guard.
fn outgoing_padding(homes: &[Mir65816AbiHome], outgoing: ByteSize) -> Result<Vec<u8>, String> {
    abi::stack::access_displacement(
        ByteOffset::new(outgoing.get()),
        ByteSize::ONE,
        ByteSize::ZERO,
    )
    .map_err(|e| e.to_string())?;
    let mut padding = Vec::new();
    let mut gap = |start, end| -> Result<(), String> {
        for offset in start..end {
            let displacement = abi::stack::access_displacement(
                ByteOffset::new(offset + 1), // bounded by the checked outgoing extent
                ByteSize::ONE,
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            padding.push(u8::try_from(displacement.get()).map_err(|_| "padding overflow")?);
        }
        Ok(())
    };
    let mut cursor = 0;
    for home in homes {
        let Mir65816AbiHome::StackArgument { offset, size, .. } = home else {
            return Err("invalid outgoing home".into());
        };
        let end = offset
            .get()
            .checked_add(size.get())
            .ok_or("argument extent overflow")?;
        if size.is_zero() || offset.get() < cursor || end > outgoing.get() {
            return Err("invalid outgoing argument range".into());
        }
        gap(cursor, offset.get())?;
        cursor = end;
    }
    gap(cursor, outgoing.get())?;
    Ok(padding)
}

#[derive(Clone, Copy)]
enum Memory {
    Stack(u32),
    DirectPage(u16),
    Absolute(u32),
    Symbol(Target, u32),
    /// The actual base slot and deferred Y displacement.
    Pointer {
        slot: u8,
        offset: u16,
    },
}

/// A complete preflight, shared by materialized and branch-only comparisons.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordComparison {
    UnsignedOrEquality,
    SignedOrder,
}

struct WordCondition {
    kind: WordComparison,
    left: WordOperand,
    left_temp: Option<TempId>,
    right: WordOperand,
    destination: u8,
    predicate: Branch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ByteOperand {
    Immediate(u8),
    Stack(u8),
}

struct ByteCondition {
    left: ByteOperand,
    right: ByteOperand,
    destination: u8,
    predicate: Branch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerOperand {
    Immediate(u32),
    Stack { low: u8, bank: u8 },
}

impl PointerOperand {
    fn bank(self) -> ByteOperand {
        match self {
            Self::Immediate(value) => ByteOperand::Immediate((value >> 16) as u8),
            Self::Stack { bank, .. } => ByteOperand::Stack(bank),
        }
    }
}

struct PointerCondition {
    left: PointerOperand,
    right: PointerOperand,
    destination: u8,
    predicate: Branch,
}

enum Condition {
    Word(WordCondition),
    Byte(ByteCondition),
    Pointer(PointerCondition),
}

impl Condition {
    fn destination(&self) -> u8 {
        match self {
            Self::Word(c) => c.destination,
            Self::Byte(c) => c.destination,
            Self::Pointer(c) => c.destination,
        }
    }
}

/// Checked word copies, with staging only when the shared plan requires it.
struct WordEdge {
    target: Label,
    copies: copies::WordCopies,
    captures: Vec<(usize, u8)>,
}

impl From<Location> for Memory {
    fn from(location: Location) -> Self {
        match location {
            Location::Stack(slot) => Self::Stack(slot.offset.into()),
            Location::DirectPage(slot) => Self::DirectPage(slot.offset),
        }
    }
}

struct Builder<'a> {
    routine: &'a Mir65816Routine,
    frame: AllocatedFrame,
    code: TrackedEmitter65816,
    blocks: BTreeMap<BlockId, Label>,
    next_block: Option<BlockId>,
    loop_x: Option<loop_x::LoopXPlan>,
}

#[cfg(test)]
pub(super) fn routine(routine: &Mir65816Routine, _trace: bool) -> Result<MachineRoutine, String> {
    routine_with_replay(
        routine,
        _trace,
        #[cfg(feature = "native65816-state-proof")]
        true,
    )
}
pub(super) fn routine_with_replay(
    routine: &Mir65816Routine,
    _trace: bool,
    #[cfg(feature = "native65816-state-proof")] replay: bool,
) -> Result<MachineRoutine, String> {
    if let Some(helper) = routine.helper {
        return arithmetic::emit(routine, helper, _trace);
    }
    let frame = AllocatedFrame::new(routine)?;
    let loop_x = loop_x::LoopXPlan::new(routine, &frame)?;
    let mut b = Builder {
        routine,
        frame,
        loop_x,
        code: TrackedEmitter65816::for_entry(routine.prologue.required_mode),
        blocks: BTreeMap::new(),
        next_block: None,
    };
    #[cfg(feature = "native65816-state-proof")]
    b.code.use_reference_planning(!replay);
    #[cfg(feature = "native65816-state-proof")]
    if _trace {
        b.code.trace();
    }
    if routine.blocks.is_empty() || !routine.blocks[0].params.is_empty() {
        return Err("routine requires an entry block without edge parameters".into());
    }
    for block in &routine.blocks {
        if b.blocks.insert(block.id, b.code.label()).is_some() {
            return Err("duplicate block identity".into());
        }
    }
    for home in b.frame.temps.values() {
        if matches!(home, Location::Stack(_)) || home.slot().width == 2 {
            b.code.register_home(*home);
        }
    }
    b.check_stack(b.frame.extent);
    b.code.op(Implied::Tcs); // TCS: checked new S in A, no write/push before the check.
    b.code.establish_body();
    for parameter in &routine.frame.parameters {
        if let Some(object) = parameter.frame_object {
            let source = b.incoming(parameter.param)?;
            let destination = b.object(object)?;
            let Mir65816AbiHome::StackArgument { size, .. } = parameter.incoming else {
                unreachable!()
            };
            b.code.a8();
            for i in 0..width(size)? {
                b.load_memory(Memory::Stack(source), u32::from(i))?;
                b.store_memory(Memory::Stack(destination), u32::from(i))?;
            }
            b.code.a16();
        }
    }
    b.code.declare_blocks(b.blocks.values().copied());
    let mut predecessors: BTreeMap<_, BTreeMap<_, usize>> = b
        .blocks
        .values()
        .map(|&label| (label, BTreeMap::new()))
        .collect();
    predecessors
        .get_mut(&b.blocks[&routine.blocks[0].id])
        .unwrap()
        .insert(None, 1);
    let mut successors = BTreeMap::new();
    for (i, block) in routine.blocks.iter().enumerate() {
        let next = match &block.terminator {
            Mir65816Terminator::Goto(edge) => vec![edge.target],
            Mir65816Terminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![else_edge.target, then_edge.target],
            Mir65816Terminator::Fallthrough => vec![
                routine
                    .blocks
                    .get(i + 1)
                    .ok_or("unresolved terminal fallthrough")?
                    .id,
            ],
            _ => vec![],
        };
        for target in &next {
            let label = b.blocks.get(target).ok_or("missing branch target label")?;
            *predecessors
                .get_mut(label)
                .unwrap()
                .entry(Some(b.blocks[&block.id]))
                .or_default() += 1;
        }
        successors.insert(block.id, next);
    }
    let mut reachable = BTreeSet::new();
    let mut pending = vec![routine.blocks[0].id];
    while let Some(id) = pending.pop() {
        if reachable.insert(b.blocks[&id]) {
            pending.extend(&successors[&id]);
        }
    }
    b.code.prove_entries(predecessors, reachable);
    if let Some(x) = &b.loop_x {
        b.code.prove_x(XContract {
            param: x.param,
            increment: x.increment.map(|id| (id, b.frame.temps[&id])),
            home: x.home,
            header: b.blocks[&x.header],
            body: b.blocks[&x.body],
            predecessors: [b.blocks[&x.preheader], b.blocks[&x.body]].into(),
        });
    }
    let sole_conditions = liveness::sole_branch_conditions(routine);
    for (index, block) in routine.blocks.iter().enumerate() {
        b.next_block = routine.blocks.get(index + 1).map(|b| b.id);
        b.code.mark(b.blocks[&block.id]);
        if let Some((last, prefix)) = block.ops.split_last() {
            for (op_index, op) in prefix.iter().enumerate() {
                let start = b.code.code().bytes.len();
                b.code.begin_source(block.id, op_index);
                b.operation(op)
                    .map_err(|e| format!("b{}: {e}", block.id.0))?;
                b.code.span(block.id, op_index, start);
            }
            let start = b.code.code().bytes.len();
            b.code.begin_source(block.id, prefix.len());
            if b.compare_branch(last, &block.terminator, &sole_conditions)
                .map_err(|e| format!("b{}: {e}", block.id.0))?
            {
                b.code
                    .fused_span(block.id, prefix.len(), start, block.ops.len());
                continue;
            }
            b.operation(last)
                .map_err(|e| format!("b{}: {e}", block.id.0))?;
            b.code.span(block.id, prefix.len(), start);
        }
        let start = b.code.code().bytes.len();
        b.code.begin_source(block.id, block.ops.len());
        b.code.a16(); // Every MIR control-flow boundary has the ABI width.
        match &block.terminator {
            Mir65816Terminator::Goto(edge) => b.edge_last(edge)?,
            Mir65816Terminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let yes = b.code.label();
                b.code.a8();
                b.value_byte(condition, 0)?;
                // Mode restoration does not change N/Z.
                b.code.a16();
                b.code.dispatch(Branch::NotEqual, yes); // BNE
                b.edge(else_edge)?;
                b.code.mark(yes);
                b.edge_last(then_edge)?;
            }
            Mir65816Terminator::Return { value, .. } => b.return_value(value.as_ref())?,
            Mir65816Terminator::Fallthrough => {
                let next = routine
                    .blocks
                    .get(index + 1)
                    .ok_or("unresolved terminal fallthrough")?;
                b.edge_last(&Mir65816Edge {
                    target: next.id,
                    args: vec![],
                })?;
            }
            Mir65816Terminator::ArithmeticFault => b.arithmetic_fault(),
            Mir65816Terminator::Exit => {
                return Err("terminal exit requires a native runtime adapter".into());
            }
        }
        b.code.span(block.id, block.ops.len(), start);
    }
    let homes = super::analysis::homes::HomeContract::from_verified(routine, &b.frame)?;
    let candidates = b.code.take_planned_loads();
    let direct = b.code.finish_selected(routine.id, &b.frame, Some(homes))?;
    #[cfg(feature = "native65816-state-proof")]
    if !replay {
        return Ok(MachineRoutine {
            id: routine.id,
            frame: b.frame,
            code: super::layout::finalize(direct, true)?,
        });
    }
    let code = super::rewrite::pilot::apply(&direct, &candidates, _trace)?;
    Ok(MachineRoutine {
        id: routine.id,
        frame: b.frame,
        code,
    })
}

impl Builder<'_> {
    fn incoming(&self, id: ParamId) -> Result<u32, String> {
        self.frame.incoming_home(self.routine, id)
    }
    fn object(&self, id: Mir65816FrameObjectId) -> Result<u32, String> {
        self.routine
            .frame
            .objects
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.stack_offset.get())
            .ok_or("unknown frame object".into())
    }
    fn parameter(&self, id: ParamId) -> Result<(u32, u8), String> {
        self.frame.parameter_home(self.routine, id)
    }
    fn temp(&self, id: TempId) -> Result<Location, String> {
        self.frame
            .temps
            .get(&id)
            .copied()
            .ok_or_else(|| format!("undefined temporary t{}", id.0))
    }
    fn displacement(&self, offset: u32, byte: u32) -> Result<u8, String> {
        let offset = offset.checked_add(byte).ok_or("stack offset overflow")?;
        abi::stack::access_displacement(
            ByteOffset::new(offset),
            ByteSize::ONE,
            ByteSize::new(self.code.delta()),
        )
        .map(|d| d.get() as u8)
        .map_err(|e| e.to_string())
    }
    fn word_displacement(&self, offset: u32) -> Result<u8, String> {
        let offset = abi::stack::access_displacement(
            ByteOffset::new(offset),
            ByteSize::new(2),
            ByteSize::new(self.code.delta()),
        )
        .map_err(|e| e.to_string())?;
        u8::try_from(offset.get()).map_err(|_| "word stack displacement overflow".into())
    }
    fn word_operand(&self, value: &Mir65816Value) -> Result<Option<WordOperand>, String> {
        self.frame
            .word_operand(self.routine, self.code.delta(), value)
    }
    fn word_binary(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<bool, String> {
        if bytes != 2 || !matches!(operation, NirBinaryOp::Add | NirBinaryOp::Sub) {
            return Ok(false);
        }
        // Preflight every operand, including the last byte after any S movement,
        // before changing either code or local accumulator-width knowledge.
        let destination = self.temp(dest)?;
        if destination.slot().width != 2 {
            return Err("word result temporary width mismatch".into());
        }
        let destination = Some(word_home(destination, self.code.delta())?);
        let left_temp = Self::word_temp(left);
        let left = self.word_operand(left)?;
        let right = self.word_operand(right)?;
        let (Some(destination), Some(left), Some(right)) = (destination, left, right) else {
            return Ok(false);
        };
        self.code.a16();
        if let Some(x) = &self.loop_x {
            if x.increment == Some(dest) {
                assert_eq!(operation, NirBinaryOp::Add);
                assert_eq!(left_temp, Some(x.param));
                assert!(matches!(right, WordOperand::Immediate(1)));
                assert_eq!(left.home().map(Location::from), Some(x.home));
                self.code
                    .increment_x_word(x.param, x.home, dest, destination.into());
                self.code.store_word(destination);
                self.remember_word(dest);
                return Ok(true);
            }
        }
        self.load_checked_word(left, left_temp);
        let subtract = operation == NirBinaryOp::Sub;
        self.code
            .op(if subtract { Implied::Sec } else { Implied::Clc }); // SEC / CLC
        match right {
            WordOperand::Immediate(value) => self.code.word(
                if subtract {
                    WordOp::SbcImm
                } else {
                    WordOp::AdcImm
                },
                value,
            ),
            WordOperand::DirectPage(offset) => self.code.byte(
                if subtract {
                    ByteOp::SbcDp
                } else {
                    ByteOp::AdcDp
                },
                offset,
            ),
            WordOperand::Stack(offset) => self.code.byte(
                if subtract {
                    ByteOp::SbcStack
                } else {
                    ByteOp::AdcStack
                },
                offset,
            ),
        }
        self.code.store_word(destination); // Capture into the verified private home.
        self.remember_word(dest);
        Ok(true)
    }
    fn word_condition(
        &self,
        dest: TempId,
        bytes: u8,
        signed: bool,
        operation: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<Option<WordCondition>, String> {
        if bytes != 2 {
            return Ok(None);
        }
        let kind = if signed && !matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne) {
            WordComparison::SignedOrder
        } else {
            WordComparison::UnsignedOrEquality
        };
        // Compare's width describes its inputs; the result is one Boolean byte.
        // Preflight everything before changing bytes, labels or mode knowledge.
        let destination = self.temp(dest)?;
        if destination.slot().width != 1 {
            return Err("comparison result temporary width mismatch".into());
        }
        let destination = match destination {
            Location::Stack(slot) => Some(self.displacement(slot.offset.into(), 0)?),
            Location::DirectPage(_) => None,
        };
        let mut left_temp = Self::word_temp(left);
        let right_temp = Self::word_temp(right);
        let left = self.word_operand(left)?;
        let right = self.word_operand(right)?;
        let (Some(destination), Some(mut left), Some(mut right)) = (destination, left, right)
        else {
            return Ok(None);
        };
        // Swapping captured values changes no source memory access or ordering.
        let mut predicate = match operation {
            NirCompareOp::Eq => Branch::Equal,      // BEQ
            NirCompareOp::Ne => Branch::NotEqual,   // BNE
            NirCompareOp::Lt => Branch::CarryClear, // BCC
            NirCompareOp::Ge => Branch::CarrySet,   // BCS
            NirCompareOp::Gt | NirCompareOp::Le => {
                std::mem::swap(&mut left, &mut right);
                left_temp = right_temp;
                if operation == NirCompareOp::Gt {
                    Branch::CarryClear
                } else {
                    Branch::CarrySet
                }
            }
        };
        if kind == WordComparison::SignedOrder {
            // Inclusive predicates use the opposite strict ordering, never Z:
            // overflow correction can produce zero for unequal inputs.
            predicate = if matches!(operation, NirCompareOp::Lt | NirCompareOp::Gt) {
                Branch::Minus
            } else {
                Branch::Plus
            };
        }
        Ok(Some(WordCondition {
            kind,
            left,
            left_temp,
            right,
            destination,
            predicate,
        }))
    }
    fn byte_operand(&self, value: &Mir65816Value) -> Result<Option<ByteOperand>, String> {
        let offset = match value {
            Mir65816Value::U8(value) => return Ok(Some(ByteOperand::Immediate(*value))),
            Mir65816Value::Null(size) if *size == ByteSize::ONE => {
                return Ok(Some(ByteOperand::Immediate(0)));
            }
            Mir65816Value::Address(value, size) if *size == ByteSize::ONE && value.value <= 255 => {
                return Ok(Some(ByteOperand::Immediate(value.value as u8)));
            }
            Mir65816Value::Temp(id, size) => {
                let location = self.temp(*id)?;
                if location.slot().width != width(*size)? {
                    return Err("temporary width mismatch".into());
                }
                if *size != ByteSize::ONE {
                    return Ok(None);
                }
                match location {
                    Location::Stack(slot) => u32::from(slot.offset),
                    Location::DirectPage(_) => return Ok(None),
                }
            }
            Mir65816Value::Param(id) => {
                let (offset, bytes) = self.parameter(*id)?;
                if bytes != 1 {
                    return Ok(None);
                }
                offset
            }
            _ => return Ok(None),
        };
        Ok(Some(ByteOperand::Stack(self.displacement(offset, 0)?)))
    }
    fn condition(
        &self,
        dest: TempId,
        bytes: u8,
        signed: bool,
        operation: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<Option<Condition>, String> {
        if bytes == 2 {
            return self
                .word_condition(dest, bytes, signed, operation, left, right)
                .map(|c| c.map(Condition::Word));
        }
        let equality = matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne);
        if !((bytes == 1 && (!signed || equality)) || (bytes == 3 && equality)) {
            return Ok(None);
        }
        let location = self.temp(dest)?;
        if location.slot().width != 1 {
            return Err("comparison result temporary width mismatch".into());
        }
        let destination = match location {
            Location::Stack(slot) => Some(self.displacement(slot.offset.into(), 0)?),
            Location::DirectPage(_) => None,
        };
        if bytes == 3 {
            let left = self.pointer_operand(left)?;
            let right = self.pointer_operand(right)?;
            let (Some(destination), Some(mut left), Some(mut right)) = (destination, left, right)
            else {
                return Ok(None);
            };
            if left == PointerOperand::Immediate(0) {
                std::mem::swap(&mut left, &mut right);
            }
            return Ok(Some(Condition::Pointer(PointerCondition {
                left,
                right,
                destination,
                predicate: if operation == NirCompareOp::Eq {
                    Branch::Equal
                } else {
                    Branch::NotEqual
                },
            })));
        }
        // Check both operands even when the first is a legal unsupported form.
        let left = self.byte_operand(left)?;
        let right = self.byte_operand(right)?;
        let (Some(destination), Some(mut left), Some(mut right)) = (destination, left, right)
        else {
            return Ok(None);
        };
        let predicate = match operation {
            NirCompareOp::Eq => Branch::Equal,
            NirCompareOp::Ne => Branch::NotEqual,
            NirCompareOp::Lt => Branch::CarryClear,
            NirCompareOp::Ge => Branch::CarrySet,
            NirCompareOp::Gt | NirCompareOp::Le => {
                std::mem::swap(&mut left, &mut right);
                if operation == NirCompareOp::Gt {
                    Branch::CarryClear
                } else {
                    Branch::CarrySet
                }
            }
        };
        Ok(Some(Condition::Byte(ByteCondition {
            left,
            right,
            destination,
            predicate,
        })))
    }
    fn pointer_operand(&self, value: &Mir65816Value) -> Result<Option<PointerOperand>, String> {
        let offset = match value {
            Mir65816Value::U24(value) => {
                if *value > 0xffffff {
                    return Err("24-bit comparison constant overflow".into());
                }
                return Ok(Some(PointerOperand::Immediate(*value)));
            }
            Mir65816Value::Null(size) if size.get() == 3 => {
                return Ok(Some(PointerOperand::Immediate(0)));
            }
            Mir65816Value::Address(value, size) if size.get() == 3 && value.value <= 0xffffff => {
                return Ok(Some(PointerOperand::Immediate(value.value as u32)));
            }
            Mir65816Value::Temp(id, size) => {
                let location = self.temp(*id)?;
                if location.slot().width != width(*size)? {
                    return Err("temporary width mismatch".into());
                }
                if size.get() != 3 {
                    return Ok(None);
                }
                match location {
                    Location::Stack(slot) => u32::from(slot.offset),
                    Location::DirectPage(_) => return Ok(None),
                }
            }
            Mir65816Value::Param(id) => {
                let (offset, bytes) = self.parameter(*id)?;
                if bytes != 3 {
                    return Ok(None);
                }
                offset
            }
            _ => return Ok(None),
        };
        // Validate both subranges before emitting anything, using the actual S delta.
        Ok(Some(PointerOperand::Stack {
            low: self.word_displacement(offset)?,
            bank: self.displacement(offset, 2)?,
        }))
    }
    fn branch_on_pointer(&mut self, condition: &PointerCondition, yes: Label, dispatch: bool) {
        self.code.barrier();
        self.code.a16();
        match condition.left {
            PointerOperand::Immediate(value) => self.code.word(WordOp::LdaImm, value as u16),
            PointerOperand::Stack { low, .. } => self.code.byte(ByteOp::LdaStack, low),
        }
        let null = condition.right == PointerOperand::Immediate(0);
        if !null {
            match condition.right {
                PointerOperand::Immediate(value) => self.code.word(WordOp::CmpImm, value as u16),
                PointerOperand::Stack { low, .. } => self.code.byte(ByteOp::CmpStack, low),
            }
        }
        // A low-word mismatch decides the result before reading the private bank byte.
        let unequal = if condition.predicate == Branch::Equal {
            let no = self.code.label();
            self.code.branch(Branch::NotEqual, no);
            Some(no)
        } else {
            if dispatch {
                self.code.dispatch(Branch::NotEqual, yes);
            } else {
                self.code.branch(Branch::NotEqual, yes);
            }
            None
        };
        self.code.a8();
        self.load_byte_operand(condition.left.bank());
        if !null {
            self.compare_byte_operand(condition.right.bank());
        }
        self.code.a16(); // Preserve bank-byte Z and give every outcome A16.
        if dispatch {
            self.code.dispatch(condition.predicate, yes);
        } else {
            self.code.branch(condition.predicate, yes);
        }
        if let Some(no) = unequal {
            self.code.mark(no);
        }
    }
    fn load_byte_operand(&mut self, operand: ByteOperand) {
        match operand {
            ByteOperand::Immediate(value) => self.code.byte(ByteOp::LdaImm, value),
            ByteOperand::Stack(offset) => self.code.byte(ByteOp::LdaStack, offset),
        }
    }
    fn compare_byte_operand(&mut self, operand: ByteOperand) {
        match operand {
            ByteOperand::Immediate(value) => self.code.byte(ByteOp::CmpImm, value),
            ByteOperand::Stack(offset) => self.code.byte(ByteOp::CmpStack, offset),
        }
    }
    fn branch_on_condition(&mut self, condition: &Condition, yes: Label, dispatch: bool) {
        match condition {
            Condition::Word(condition) => self.branch_on_word(condition, yes, dispatch),
            Condition::Pointer(condition) => self.branch_on_pointer(condition, yes, dispatch),
            Condition::Byte(condition) => {
                self.code.barrier(); // Retain the original operation's value/flag barrier.
                self.code.a8();
                self.load_byte_operand(condition.left);
                self.compare_byte_operand(condition.right);
                if dispatch {
                    self.code.a16(); // REP preserves the A8 CMP's C/Z.
                    self.code.dispatch(condition.predicate, yes);
                } else {
                    self.code.branch(condition.predicate, yes);
                }
            }
        }
    }
    fn branch_on_word(&mut self, condition: &WordCondition, yes: Label, dispatch: bool) {
        if condition.kind == WordComparison::SignedOrder {
            self.code.barrier(); // Retain the fallback's forwarding boundary.
        }
        self.code.a16();
        self.load_checked_word(condition.left, condition.left_temp);
        match condition.kind {
            WordComparison::UnsignedOrEquality => match condition.right {
                WordOperand::Immediate(value) => self.code.word(WordOp::CmpImm, value),
                WordOperand::Stack(offset) => self.code.byte(ByteOp::CmpStack, offset),
                WordOperand::DirectPage(offset) => self.code.byte(ByteOp::CmpDp, offset),
            },
            WordComparison::SignedOrder => {
                self.code.op(Implied::Sec);
                match condition.right {
                    WordOperand::Immediate(value) => self.code.word(WordOp::SbcImm, value),
                    WordOperand::Stack(offset) => self.code.byte(ByteOp::SbcStack, offset),
                    WordOperand::DirectPage(offset) => self.code.byte(ByteOp::SbcDp, offset),
                }
                let corrected = self.code.label();
                self.code.branch(Branch::OverflowClear, corrected);
                self.code.word(WordOp::EorImm, 0x8000);
                self.code.mark(corrected);
            }
        }
        if dispatch {
            self.code.dispatch(condition.predicate, yes);
        } else {
            self.code.branch(condition.predicate, yes);
        } // Consume CMP's C/Z or the corrected subtraction's N immediately.
    }
    fn compare_branch(
        &mut self,
        op: &Mir65816Op,
        terminator: &Mir65816Terminator,
        sole_conditions: &BTreeSet<TempId>,
    ) -> Result<bool, String> {
        let (
            Mir65816Op::Compare {
                dest,
                width: bytes,
                signed,
                operation,
                left,
                right,
            },
            Mir65816Terminator::Branch {
                condition: Mir65816Value::Temp(id, size),
                then_edge,
                else_edge,
            },
        ) = (op, terminator)
        else {
            return Ok(false);
        };
        if dest != id || *size != ByteSize::ONE || !sole_conditions.contains(id) {
            return Ok(false);
        }
        let Some(condition) =
            self.condition(*dest, width(*bytes)?, *signed, *operation, left, right)?
        else {
            return Ok(false);
        };
        let yes = self.code.label();
        if let Some(x) = &self.loop_x
            && x.condition == *dest
        {
            self.code.a16();
            self.code.compare_x_word(x.param, x.home, x.threshold);
            self.code.dispatch(Branch::CarryClear, yes);
        } else {
            self.branch_on_condition(&condition, yes, true);
        }
        // Each edge still stages parallel arguments before writing destinations.
        self.edge(else_edge)?;
        self.code.mark(yes);
        self.edge_last(then_edge)?;
        Ok(true)
    }
    fn native_compare(
        &mut self,
        dest: TempId,
        bytes: u8,
        signed: bool,
        operation: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<bool, String> {
        let Some(condition) = self.condition(dest, bytes, signed, operation, left, right)? else {
            return Ok(false);
        };
        let yes = self.code.label();
        let done = self.code.label();
        self.branch_on_condition(&condition, yes, false);
        self.code.a8();
        self.code.byte(ByteOp::LdaImm, 0);
        self.code.jump(done);
        self.code.mark(yes);
        self.code.a8();
        self.code.byte(ByteOp::LdaImm, 1);
        self.code.mark(done);
        self.code.a8(); // Joins never inherit the fallthrough mode knowledge.
        self.code.byte(ByteOp::StaStack, condition.destination());
        Ok(true)
    }
    fn load_memory(&mut self, memory: Memory, byte: u32) -> Result<(), String> {
        self.memory(
            ByteOp::LdaStack,
            LongOp::Lda,
            ByteOp::LdaIndirectY,
            memory,
            byte,
        )
    }
    fn store_memory(&mut self, memory: Memory, byte: u32) -> Result<(), String> {
        self.memory(
            ByteOp::StaStack,
            LongOp::Sta,
            ByteOp::StaIndirectY,
            memory,
            byte,
        )
    }
    fn memory(
        &mut self,
        stack: ByteOp,
        long: LongOp,
        indirect: ByteOp,
        memory: Memory,
        byte: u32,
    ) -> Result<(), String> {
        match memory {
            Memory::Stack(offset) => self.code.byte(stack, self.displacement(offset, byte)?),
            Memory::DirectPage(offset) => {
                let offset = u8::try_from(u32::from(offset) + byte)
                    .map_err(|_| "direct-page offset overflow")?;
                self.code.byte(
                    if stack == ByteOp::LdaStack {
                        ByteOp::LdaDp
                    } else {
                        ByteOp::StaDp
                    },
                    offset,
                );
            }
            Memory::Absolute(address) => self
                .code
                .long(long, address.checked_add(byte).ok_or("address overflow")?)?,
            Memory::Symbol(target, offset) => self.code.reference(
                if long == LongOp::Lda {
                    ReferenceOp::LdaLong
                } else {
                    ReferenceOp::StaLong
                },
                target,
                offset.checked_add(byte).ok_or("address offset overflow")?,
                None,
            ),
            Memory::Pointer { slot, offset } => {
                self.code.word(
                    WordOp::LdyImm,
                    u16::try_from(u32::from(offset) + byte)
                        .map_err(|_| "indirect displacement exceeds Y")?,
                ); // LDY
                self.code.byte(indirect, slot); // LDA/STA [PTR],Y: linear 24-bit access
            }
        }
        Ok(())
    }
    fn check_transfer(&self, source: Memory, destination: Memory, bytes: u8) -> Result<(), String> {
        for memory in [source, destination] {
            match memory {
                Memory::Stack(offset) => {
                    self.displacement(offset, u32::from(bytes) - 1)?;
                }
                Memory::Absolute(address)
                    if address
                        .checked_add(u32::from(bytes) - 1)
                        .is_none_or(|end| end >= 1 << 24) =>
                {
                    return Err("24-bit instruction address overflow".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
    /// Copy exactly the scalar extent. Ordinary accesses may use word pairs;
    /// volatile accesses retain their individual ascending byte transfers.
    fn transfer(
        &mut self,
        source: Memory,
        destination: Memory,
        bytes: u8,
        wide: bool,
    ) -> Result<(), String> {
        self.check_transfer(source, destination, bytes)?;
        // Private frame transfers can stay in A16 for a three-byte value by
        // overlapping the two words. Never touch a fourth byte, and never
        // duplicate a read/write through an external or indirect address.
        if wide
            && bytes == 3
            && match (source, destination) {
                (Memory::Stack(src), Memory::Stack(dst)) => src == dst || src.abs_diff(dst) >= 3,
                (Memory::DirectPage(src), Memory::DirectPage(dst)) => {
                    src == dst || src.abs_diff(dst) >= 3
                }
                (Memory::Stack(_), Memory::DirectPage(_))
                | (Memory::DirectPage(_), Memory::Stack(_)) => true,
                _ => false,
            }
        {
            self.code.a16();
            for byte in [0, 1] {
                self.load_memory(source, byte)?;
                self.store_memory(destination, byte)?;
            }
            return Ok(());
        }
        let mut byte = 0;
        while byte < bytes {
            let word = wide && byte + 1 < bytes;
            if word {
                self.code.a16();
            } else {
                self.code.a8();
            }
            self.load_memory(source, byte.into())?;
            self.store_memory(destination, byte.into())?;
            byte += if word { 2 } else { 1 };
        }
        Ok(())
    }
    fn value_memory(&self, value: &Mir65816Value) -> Result<Option<Memory>, String> {
        Ok(match value {
            Mir65816Value::Temp(id, bytes) => {
                let slot = self.temp(*id)?;
                if slot.slot().width != width(*bytes)? {
                    return Err("temporary width mismatch".into());
                }
                Some(slot.into())
            }
            Mir65816Value::Param(id) => Some(Memory::Stack(self.parameter(*id)?.0)),
            _ => None,
        })
    }
    fn value_width(&self, value: &Mir65816Value) -> Result<u8, String> {
        self.frame.value_width(self.routine, value)
    }
    /// A8 byte load, leaving carry intact for multi-byte arithmetic.
    fn value_byte(&mut self, value: &Mir65816Value, byte: u8) -> Result<(), String> {
        // NIR permits narrow operands (notably loop-step constants). Values are
        // zero-extended here; signed widening is an explicit Cast operation.
        if byte >= self.value_width(value)? {
            self.code.byte(ByteOp::LdaImm, 0);
            return Ok(());
        }
        match value {
            Mir65816Value::U8(v) => self.code.byte(ByteOp::LdaImm, *v),
            Mir65816Value::U16(v) => self.code.byte(ByteOp::LdaImm, (*v >> (byte * 8)) as u8),
            Mir65816Value::U24(v) | Mir65816Value::U32(v) => {
                self.code.byte(ByteOp::LdaImm, (*v >> (byte * 8)) as u8)
            }
            Mir65816Value::Null(_) => self.code.byte(ByteOp::LdaImm, 0),
            Mir65816Value::Address(v, _) => self
                .code
                .byte(ByteOp::LdaImm, (v.value >> (byte * 8)) as u8),
            Mir65816Value::Temp(id, w) => {
                let slot = self.temp(*id)?;
                if slot.slot().width != width(*w)? {
                    return Err("temporary width mismatch".into());
                }
                self.load_memory(slot.into(), byte.into())?;
            }
            Mir65816Value::Param(id) => {
                self.load_memory(Memory::Stack(self.parameter(*id)?.0), byte.into())?
            }
            Mir65816Value::StaticAddress(id, _) => self.code.reference(
                ReferenceOp::LdaByte,
                Target::Data(Mir65816DataId::Static(*id)),
                0,
                Some(byte),
            ),
            Mir65816Value::GlobalAddress(id, _) => self.code.reference(
                ReferenceOp::LdaByte,
                Target::Data(Mir65816DataId::Global(*id)),
                0,
                Some(byte),
            ),
            Mir65816Value::RoutineAddress(id, _) => self.code.reference(
                ReferenceOp::LdaByte,
                Target::Routine(RoutineId(*id)),
                0,
                Some(byte),
            ),
        }
        Ok(())
    }
    fn save_byte(&mut self, dest: TempId, byte: u8) -> Result<(), String> {
        let slot = self.temp(dest)?;
        if byte >= slot.slot().width {
            return Err("temporary write exceeds its width".into());
        }
        self.store_memory(slot.into(), byte.into())
    }
    fn pointer_value(&mut self, value: &Mir65816Value, scratch: u8) -> Result<(), String> {
        let bytes = self.value_width(value)?;
        if bytes >= 2 {
            if let Some(memory) = self.value_memory(value)? {
                if let Memory::Stack(offset) = memory {
                    // A word's trailing byte must also fit the ABI's d,S
                    // range, including transient outgoing-call reservations.
                    self.displacement(offset, u32::from(bytes.min(3)) - 1)?;
                }
                self.code.a16();
                self.load_memory(memory, 0)?;
                self.code.byte(ByteOp::StaDp, scratch);
                if bytes >= 3 {
                    // Both source and scratch have three owned bytes. Reading
                    // the overlapping word avoids a bank-byte mode switch.
                    self.load_memory(memory, 1)?;
                    self.code.byte(ByteOp::StaDp, scratch + 1);
                    return Ok(());
                }
                self.code.a8();
                self.value_byte(value, 2)?;
                self.code.byte(ByteOp::StaDp, scratch + 2);
                return Ok(());
            }
        }
        self.code.a8();
        for i in 0..3 {
            if i < bytes {
                self.value_byte(value, i)?;
            } else {
                self.code.byte(ByteOp::LdaImm, 0);
            }
            self.code.byte(ByteOp::StaDp, scratch + i);
        }
        Ok(())
    }
    fn prepare_address(&mut self, address: &Mir65816Address) -> Result<Memory, String> {
        let displacement = address.displacement.get();
        let memory = match &address.base {
            Mir65816AddressBase::AutomaticFrame(id) => Memory::Stack(self.object(*id)?),
            Mir65816AddressBase::Parameter(id) => Memory::Stack(self.parameter(*id)?.0),
            Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(a)) => {
                Memory::Absolute(
                    u32::try_from(a.value).map_err(|_| "absolute address exceeds 24 bits")?,
                )
            }
            Mir65816AddressBase::External(Mir65816ExternalAddress::Global(id))
            | Mir65816AddressBase::Static(NirStorageId::Global(id)) => {
                Memory::Symbol(Target::Data(Mir65816DataId::Global(*id)), 0)
            }
            Mir65816AddressBase::Static(_) => {
                return Err("unresolved static invocation storage".into());
            }
            Mir65816AddressBase::Indirect(value) => {
                if self.value_width(value)? != 3 {
                    return Err("indirect address requires a 24-bit pointer".into());
                }
                if let Mir65816Value::Temp(id, _) = value
                    && let Location::DirectPage(slot) = self.temp(*id)?
                {
                    if address.index.is_some() || displacement > u32::from(u16::MAX) - 3 {
                        return Err("unmodelled resident pointer addressing".into());
                    }
                    return Ok(Memory::Pointer {
                        slot: slot.offset as u8,
                        offset: displacement as u16,
                    });
                }
                self.pointer_value(value, PTR)?;
                Memory::Pointer {
                    slot: PTR,
                    offset: 0,
                }
            }
        };
        if address.index.is_none() && !matches!(memory, Memory::Pointer { .. }) {
            return Ok(match memory {
                Memory::Stack(offset) => Memory::Stack(
                    offset
                        .checked_add(displacement)
                        .ok_or("stack offset overflow")?,
                ),
                Memory::Absolute(a) => Memory::Absolute(
                    a.checked_add(displacement)
                        .ok_or("absolute offset overflow")?,
                ),
                Memory::Symbol(t, offset) => Memory::Symbol(
                    t,
                    offset
                        .checked_add(displacement)
                        .ok_or("symbol offset overflow")?,
                ),
                Memory::Pointer { .. } | Memory::DirectPage(_) => unreachable!(),
            });
        }
        if !matches!(memory, Memory::Pointer { .. }) {
            self.code.a8();
            self.address_to_pointer(memory)?;
        }
        if let Some(index) = &address.index {
            // Constant-stride scaling in the full 24-bit address domain. This
            // uses only scratch and does not introduce an unqualified helper.
            self.pointer_value(&index.value, INDEX)?;
            self.code.a8();
            let stride = index.stride.get();
            if stride == 0 || stride >= 0x1000000 {
                return Err("unsupported index stride".into());
            }
            for bit in 0..(32 - stride.leading_zeros()) {
                if stride & (1 << bit) != 0 {
                    self.code.op(Implied::Clc);
                    for i in 0..3 {
                        self.code.byte(ByteOp::LdaDp, PTR + i);
                        self.code.byte(ByteOp::AdcDp, INDEX + i);
                        self.code.byte(ByteOp::StaDp, PTR + i);
                    }
                }
                self.code.byte(ByteOp::AslDp, INDEX); // ASL / ROL, low byte first
                self.code.byte(ByteOp::RolDp, INDEX + 1);
                self.code.byte(ByteOp::RolDp, INDEX + 2);
            }
        }
        if displacement >= 0x1000000 {
            return Err("pointer displacement exceeds 24 bits".into());
        }
        // Leave room for every byte of the largest scalar (four bytes).
        // AddressOf and aggregate copies explicitly materialize this offset.
        if displacement <= u32::from(u16::MAX) - 3 {
            return Ok(Memory::Pointer {
                slot: PTR,
                offset: displacement as u16,
            });
        }
        if displacement != 0 {
            self.code.a8();
            self.code.op(Implied::Clc);
            for i in 0..3 {
                self.code.byte(ByteOp::LdaDp, PTR + i);
                self.code
                    .byte(ByteOp::AdcImm, (displacement >> (i * 8)) as u8);
                self.code.byte(ByteOp::StaDp, PTR + i);
            }
        }
        Ok(Memory::Pointer {
            slot: PTR,
            offset: 0,
        })
    }
    fn address_to_pointer(&mut self, memory: Memory) -> Result<(), String> {
        self.code.a8();
        match memory {
            Memory::Stack(offset) => {
                let displacement = self.displacement(offset, 0)?;
                self.code.a16();
                self.code.op(Implied::Tsc);
                self.code.op(Implied::Clc); // TSC / CLC
                self.code.word(WordOp::AdcImm, displacement.into());
                self.code.byte(ByteOp::StaDp, PTR);
                self.code.a8();
                self.code.byte(ByteOp::LdaImm, 0);
                self.code.byte(ByteOp::StaDp, PTR + 2);
            }
            Memory::Absolute(a) => {
                if a >= 0x1000000 {
                    return Err("24-bit address overflow".into());
                }
                for i in 0..3 {
                    self.code.byte(ByteOp::LdaImm, (a >> (i * 8)) as u8);
                    self.code.byte(ByteOp::StaDp, PTR + i);
                }
            }
            Memory::Symbol(t, offset) => {
                for i in 0..3 {
                    self.code
                        .reference(ReferenceOp::LdaByte, t, offset, Some(i));
                    self.code.byte(ByteOp::StaDp, PTR + i);
                }
            }
            Memory::DirectPage(_) => return Err("resident scratch address cannot escape".into()),
            Memory::Pointer { slot, offset } => {
                if slot != PTR {
                    return Err("resident pointer cannot use generic address scratch".into());
                }
                if offset != 0 {
                    self.pointer_step(PTR, false, offset.into());
                }
            }
        }
        Ok(())
    }
    fn check_stack(&mut self, bytes: u16) {
        self.code.barrier();
        // A/X/Y are caller-clobbered. X retains the unchanged S for the raw
        // overflow adapter. Neither branch changes I, D, DBR or the stack.
        let within = self.code.label();
        let fault = self.code.label();
        let done = self.code.label();
        self.code.op(Implied::Tsc);
        self.code.op(Implied::Tax); // TSC / TAX
        self.code
            .byte(ByteOp::CmpDp, abi::generated::DP_STACK_CEILING_OFFSET as u8);
        self.code.dispatch(Branch::CarryClear, within);
        self.code.dispatch(Branch::Equal, within);
        self.code.jump(fault);
        self.code.mark(within);
        self.code.op(Implied::Sec);
        self.code.word(WordOp::SbcImm, bytes); // SEC / SBC
        self.code.dispatch(Branch::CarryClear, fault);
        self.code
            .byte(ByteOp::CmpDp, abi::generated::DP_STACK_FLOOR_OFFSET as u8);
        self.code.dispatch(Branch::CarrySet, done);
        self.code.mark(fault);
        self.code.word(WordOp::LdaImm, bytes);
        self.code
            .reference(ReferenceOp::Jml, Target::StackOverflow, 0, None);
        self.code.mark(done);
    }
    fn reserve(&mut self, bytes: u16) {
        self.code.barrier();
        self.code.op(Implied::Tsc);
        self.code.op(Implied::Sec);
        self.code.word(WordOp::SbcImm, bytes);
        self.code.op(Implied::Tcs);
    }
    fn release(&mut self, bytes: u16, preserve_result: bool) {
        self.code.barrier();
        if bytes != 0 {
            // TAY; TSC; CLC; ADC #bytes; TCS; TYA. Preserve the entire A/X result.
            if preserve_result {
                self.code.op(Implied::Tay);
            }
            self.code.op(Implied::Tsc);
            self.code.op(Implied::Clc);
            self.code.word(WordOp::AdcImm, bytes);
            self.code.op(Implied::Tcs);
            if preserve_result {
                self.code.op(Implied::Tya);
            }
        }
    }
    fn staging(&self, n: usize, bytes: u8) -> Result<u8, String> {
        let slot = self
            .frame
            .edge_copies
            .get(n)
            .ok_or("missing edge staging slot")?;
        if !(bytes..=4).contains(&slot.width) {
            return Err("invalid edge staging slot width".into());
        }
        abi::stack::access_displacement(
            ByteOffset::new(slot.offset.into()),
            ByteSize::new(bytes.into()),
            ByteSize::new(self.code.delta()),
        )
        .map(|d| d.get() as u8)
        .map_err(|e| e.to_string())
    }
    fn word_edge(&self, edge: &Mir65816Edge) -> Result<Option<WordEdge>, String> {
        let copies = self
            .frame
            .word_copies(self.routine, edge, self.code.delta())?;
        let target = *self
            .blocks
            .get(&edge.target)
            .ok_or("missing branch target label")?;
        // Preflight fallback capacity as well, before any prefix or copy writes.
        let Some(copies) = copies else {
            for (i, bytes) in self
                .frame
                .edge_widths(self.routine, edge)?
                .into_iter()
                .enumerate()
            {
                self.staging(i, bytes)?;
            }
            return Ok(None);
        };
        let captures = self.frame.word_staging(&copies, self.code.delta())?;
        Ok(Some(WordEdge {
            target,
            copies,
            captures,
        }))
    }
    fn edge_load(&mut self, source: WordOperand) {
        match source {
            WordOperand::Immediate(value) => self.code.word(WordOp::LdaImm, value),
            WordOperand::Stack(offset) => self.code.byte(ByteOp::LdaStack, offset),
            WordOperand::DirectPage(offset) => self.code.byte(ByteOp::LdaDp, offset),
        }
    }
    fn emit_word_edge(&mut self, edge: WordEdge, fallthrough: bool) {
        self.code.barrier();
        self.code.a16();
        if let Some((order, repair)) = edge.copies.direct_emission() {
            for i in order {
                let (source, destination) = edge.copies.moves[i];
                self.edge_load(source);
                self.code.store_word(destination);
            }
            if repair {
                // Retain full A and N/Z of the original final assignment.
                self.edge_load(edge.copies.moves.last().unwrap().1.operand());
            }
        } else {
            for &(i, at) in &edge.captures {
                self.edge_load(edge.copies.moves[i].0);
                self.code.byte(ByteOp::StaStack, at);
            }
            let mut captures = edge.captures.iter().peekable();
            for (i, &(source, destination)) in edge.copies.moves.iter().enumerate() {
                let source = if captures.peek().is_some_and(|&&(j, _)| j == i) {
                    WordOperand::Stack(captures.next().unwrap().1)
                } else {
                    source
                };
                self.edge_load(source);
                self.code.store_word(destination);
            }
        }
        self.finish_edge(edge.target, fallthrough);
    }
    fn edge(&mut self, edge: &Mir65816Edge) -> Result<(), String> {
        self.edge_transfer(edge, false)
    }
    fn edge_last(&mut self, edge: &Mir65816Edge) -> Result<(), String> {
        self.edge_transfer(edge, self.next_block == Some(edge.target))
    }
    fn finish_edge(&mut self, target: Label, fallthrough: bool) {
        if self
            .loop_x
            .as_ref()
            .is_some_and(|x| self.blocks[&x.header] == target)
        {
            self.code.refresh_x();
        }
        if fallthrough {
            self.code.fallthrough(target);
        } else {
            self.code.jump(target);
        }
    }
    fn edge_transfer(&mut self, edge: &Mir65816Edge, fallthrough: bool) -> Result<(), String> {
        self.code.barrier();
        if let Some(word) = self.word_edge(edge)? {
            self.emit_word_edge(word, fallthrough);
            return Ok(());
        }
        let block = self
            .routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .ok_or("unknown branch target")?;
        if edge.args.len() != block.params.len() {
            return Err("edge argument count mismatch".into());
        }
        if edge.args.is_empty() {
            let target = *self
                .blocks
                .get(&edge.target)
                .ok_or("missing branch target label")?;
            // No copies need A8. Keep the successor's A16 contract, including
            // at branch labels where local mode knowledge has been invalidated.
            self.code.a16();
            self.finish_edge(target, fallthrough);
            return Ok(());
        }
        self.code.a8();
        // Save every source before assigning any destination: parallel copies
        // stay correct for loops that swap or rotate live values.
        for (n, (value, &(_, bytes))) in edge.args.iter().zip(&block.params).enumerate() {
            if self.value_width(value)? != width(bytes)? {
                return Err("edge argument width mismatch".into());
            }
            let slot = self.frame.edge_copies[n];
            for i in 0..width(bytes)? {
                self.value_byte(value, i)?;
                self.store_memory(Memory::Stack(slot.offset.into()), i.into())?;
            }
        }
        for (n, &(dest, bytes)) in block.params.iter().enumerate() {
            let slot = self.frame.edge_copies[n];
            for i in 0..width(bytes)? {
                self.load_memory(Memory::Stack(slot.offset.into()), i.into())?;
                self.save_byte(dest, i)?;
            }
        }
        self.code.a16();
        self.finish_edge(self.blocks[&edge.target], fallthrough);
        Ok(())
    }
    fn operation(&mut self, op: &Mir65816Op) -> Result<(), String> {
        if let Mir65816Op::Call {
            target,
            args,
            result,
            plan,
            ..
        } = op
        {
            self.code.barrier();
            self.code.a16();
            return self.call(target, args, *result, plan);
        }
        if let Mir65816Op::Binary {
            dest,
            width: bytes,
            operation,
            left,
            right,
            ..
        } = op
            && self.word_binary(*dest, width(*bytes)?, *operation, left, right)?
        {
            return Ok(());
        }
        if let Mir65816Op::Compare {
            dest,
            width: bytes,
            signed,
            operation,
            left,
            right,
        } = op
            && self.native_compare(*dest, width(*bytes)?, *signed, *operation, left, right)?
        {
            return Ok(());
        }
        if let Mir65816Op::Load {
            dest,
            width: bytes,
            address,
            volatile,
        } = op
            && (self.incoming_word_load(*dest, width(*bytes)?, address, *volatile)?
                || self.frame_word_load(*dest, width(*bytes)?, address, *volatile)?)
        {
            return Ok(());
        }
        if !matches!(op, Mir65816Op::Store { .. }) {
            self.code.barrier();
        }
        if !matches!(op, Mir65816Op::Load { .. } | Mir65816Op::Store { .. }) {
            self.code.a8();
        }
        match op {
            Mir65816Op::Load {
                dest,
                width: bytes,
                address,
                volatile,
            } => {
                let memory = self.prepare_address(address)?;
                let slot = self.temp(*dest)?;
                if slot.slot().width != width(*bytes)? {
                    return Err("load temporary width mismatch".into());
                }
                self.transfer(memory, slot.into(), slot.slot().width, !volatile)?;
                if !volatile && bytes.get() == 2 && Self::direct_word_address(address) {
                    self.remember_word(*dest);
                }
            }
            Mir65816Op::Store {
                address,
                value,
                width: bytes,
                volatile,
            } => {
                if self.word_store(address, value, width(*bytes)?, *volatile)? {
                    return Ok(());
                }
                self.code.barrier();
                let memory = self.prepare_address(address)?;
                let bytes = width(*bytes)?;
                if let Some(source) = self.value_memory(value)?
                    && self.value_width(value)? >= bytes
                {
                    self.transfer(source, memory, bytes, !volatile)?;
                } else {
                    self.code.a8();
                    for i in 0..bytes {
                        self.value_byte(value, i)?;
                        self.store_memory(memory, i.into())?;
                    }
                }
            }
            Mir65816Op::AddressOf {
                dest,
                address,
                width: bytes,
            } => {
                if bytes.get() != 3 {
                    return Err("address result must retain 24 bits".into());
                }
                let memory = self.prepare_address(address)?;
                self.address_to_pointer(memory)?;
                for i in 0..3 {
                    self.code.byte(ByteOp::LdaDp, PTR + i);
                    self.save_byte(*dest, i)?;
                }
            }
            Mir65816Op::Unary {
                dest,
                width: bytes,
                operation,
                value,
            } => {
                if *operation == NirUnaryOp::Neg {
                    self.code.op(Implied::Sec);
                }
                for i in 0..width(*bytes)? {
                    self.value_byte(value, i)?;
                    if *operation == NirUnaryOp::Neg {
                        self.code.byte(ByteOp::StaDp, RIGHT);
                        self.code.byte(ByteOp::LdaImm, 0);
                        self.code.byte(ByteOp::SbcDp, RIGHT);
                    }
                    self.save_byte(*dest, i)?;
                }
            }
            Mir65816Op::Cast {
                dest,
                from,
                from_signed,
                to,
                value,
                ..
            } => {
                let from = width(*from)?;
                let to = width(*to)?;
                for i in 0..to.min(from) {
                    self.value_byte(value, i)?;
                    self.save_byte(*dest, i)?;
                }
                if to > from {
                    if *from_signed {
                        let positive = self.code.label();
                        let ready = self.code.label();
                        self.value_byte(value, from - 1)?;
                        self.code.branch(Branch::Plus, positive); // BPL
                        self.code.byte(ByteOp::LdaImm, 0xff);
                        self.code.jump(ready);
                        self.code.mark(positive);
                        self.code.byte(ByteOp::LdaImm, 0);
                        self.code.mark(ready);
                    } else {
                        self.code.byte(ByteOp::LdaImm, 0);
                    }
                    for i in from..to {
                        self.save_byte(*dest, i)?;
                    }
                }
            }
            Mir65816Op::Binary {
                dest,
                width: bytes,
                operation,
                left,
                right,
                ..
            } => self.binary(*dest, width(*bytes)?, *operation, left, right)?,
            Mir65816Op::PointerOffset {
                dest,
                width: bytes,
                base,
                offset,
                subtract,
                offset_signed,
            } => {
                let offset_bytes = self.value_width(offset)?;
                self.code.op(if *subtract {
                    Implied::Sec
                } else {
                    Implied::Clc
                });
                for i in 0..width(*bytes)? {
                    if *offset_signed && i >= offset_bytes {
                        let positive = self.code.label();
                        let ready = self.code.label();
                        self.value_byte(offset, offset_bytes - 1)?;
                        self.code.branch(Branch::Plus, positive);
                        self.code.byte(ByteOp::LdaImm, 0xff);
                        self.code.jump(ready);
                        self.code.mark(positive);
                        self.code.byte(ByteOp::LdaImm, 0);
                        self.code.mark(ready);
                    } else {
                        self.value_byte(offset, i)?;
                    }
                    self.code.byte(ByteOp::StaDp, RIGHT);
                    self.value_byte(base, i)?;
                    self.code.byte(
                        if *subtract {
                            ByteOp::SbcDp
                        } else {
                            ByteOp::AdcDp
                        },
                        RIGHT,
                    );
                    self.save_byte(*dest, i)?;
                }
            }
            Mir65816Op::Compare {
                dest,
                width: bytes,
                signed,
                operation,
                left,
                right,
            } => self.compare(*dest, width(*bytes)?, *signed, *operation, left, right)?,
            Mir65816Op::Copy {
                destination,
                source,
                bytes,
                overlap_safe,
                destination_volatile,
                source_volatile,
            } => {
                if *destination_volatile || *source_volatile {
                    return Err(
                        "volatile aggregate copy requires an explicit byte-access protocol".into(),
                    );
                }
                self.copy(destination, source, bytes.get(), *overlap_safe)?;
            }
            Mir65816Op::Call { .. } => unreachable!(),
        }
        Ok(())
    }
    fn binary(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<(), String> {
        if matches!(operation, NirBinaryOp::Lsh | NirBinaryOp::Rsh) {
            return self.shift(dest, bytes, operation == NirBinaryOp::Lsh, left, right);
        }
        let opcode = match operation {
            NirBinaryOp::Add => {
                self.code.op(Implied::Clc);
                ByteOp::AdcDp
            }
            NirBinaryOp::Sub => {
                self.code.op(Implied::Sec);
                ByteOp::SbcDp
            }
            NirBinaryOp::And => ByteOp::AndDp,
            NirBinaryOp::Or => ByteOp::OraDp,
            NirBinaryOp::Xor => ByteOp::EorDp,
            _ => {
                return Err(format!(
                    "native emission does not support integer {operation:?}"
                ));
            }
        };
        for i in 0..bytes {
            self.value_byte(right, i)?;
            self.code.byte(ByteOp::StaDp, RIGHT);
            self.value_byte(left, i)?;
            self.code.byte(opcode, RIGHT);
            self.save_byte(dest, i)?;
        }
        Ok(())
    }
    fn shift(
        &mut self,
        dest: TempId,
        bytes: u8,
        left_shift: bool,
        left: &Mir65816Value,
        count: &Mir65816Value,
    ) -> Result<(), String> {
        let zero = self.code.label();
        let ready = self.code.label();
        let loop_start = self.code.label();
        for i in 0..bytes {
            self.value_byte(left, i)?;
            self.code.byte(ByteOp::StaDp, RESULT + i);
        }
        for i in 1..self.value_width(count)? {
            self.value_byte(count, i)?;
            self.code.branch(Branch::NotEqual, zero);
        }
        self.value_byte(count, 0)?;
        self.code.byte(ByteOp::CmpImm, bytes * 8);
        self.code.branch(Branch::CarrySet, zero);
        self.code.byte(ByteOp::CmpImm, 0);
        self.code.branch(Branch::Equal, ready);
        self.code.a16();
        self.code.word(WordOp::AndImm, 0xff);
        self.code.op(Implied::Tax);
        self.code.a8();
        self.code.mark(loop_start);
        if left_shift {
            self.code.byte(ByteOp::AslDp, RESULT);
            for i in 1..bytes {
                self.code.byte(ByteOp::RolDp, RESULT + i);
            }
        } else {
            self.code.byte(ByteOp::LsrDp, RESULT + bytes - 1);
            for i in (0..bytes - 1).rev() {
                self.code.byte(ByteOp::RorDp, RESULT + i);
            }
        }
        self.code.op(Implied::Dex);
        self.code.branch(Branch::NotEqual, loop_start);
        self.code.jump(ready);
        self.code.mark(zero);
        self.code.byte(ByteOp::LdaImm, 0);
        for i in 0..bytes {
            self.code.byte(ByteOp::StaDp, RESULT + i);
        }
        self.code.mark(ready);
        for i in 0..bytes {
            self.code.byte(ByteOp::LdaDp, RESULT + i);
            self.save_byte(dest, i)?;
        }
        Ok(())
    }
    fn pointer_step(&mut self, pointer: u8, subtract: bool, amount: u32) {
        self.code
            .op(if subtract { Implied::Sec } else { Implied::Clc });
        for i in 0..3 {
            self.code.byte(ByteOp::LdaDp, pointer + i);
            self.code.byte(
                if subtract {
                    ByteOp::SbcImm
                } else {
                    ByteOp::AdcImm
                },
                (amount >> (i * 8)) as u8,
            );
            self.code.byte(ByteOp::StaDp, pointer + i);
        }
    }
    fn copy(
        &mut self,
        destination: &Mir65816Address,
        source: &Mir65816Address,
        bytes: u32,
        overlap_safe: bool,
    ) -> Result<(), String> {
        if bytes == 0 {
            return Ok(());
        }
        if bytes >= 1 << 24 {
            return Err("copy extent exceeds native address space".into());
        }
        let memory = self.prepare_address(destination)?;
        self.address_to_pointer(memory)?;
        for i in 0..3 {
            self.code.byte(ByteOp::LdaDp, PTR + i);
            self.code.byte(ByteOp::StaDp, 24 + i);
        }
        let memory = self.prepare_address(source)?;
        self.address_to_pointer(memory)?;
        for i in 0..3 {
            self.code.byte(ByteOp::LdaDp, PTR + i);
            self.code.byte(ByteOp::StaDp, 3 + i);
            self.code.byte(ByteOp::LdaDp, 24 + i);
            self.code.byte(ByteOp::StaDp, PTR + i);
        }
        let forward = self.code.label();
        let backward = self.code.label();
        let done = self.code.label();
        if overlap_safe {
            for i in (0..3).rev() {
                self.code.byte(ByteOp::LdaDp, PTR + i);
                self.code.byte(ByteOp::CmpDp, 3 + i);
                self.code.branch(Branch::CarryClear, forward);
                self.code.branch(Branch::NotEqual, backward);
            }
            self.code.jump(done); // identical source/destination
            self.code.mark(backward);
            self.pointer_step(PTR, false, bytes - 1);
            self.pointer_step(3, false, bytes - 1);
            self.copy_loop(bytes, true);
            self.code.jump(done);
        }
        self.code.mark(forward);
        self.copy_loop(bytes, false);
        self.code.mark(done);
        Ok(())
    }
    fn copy_loop(&mut self, bytes: u32, backward: bool) {
        for i in 0..3 {
            self.code.byte(ByteOp::LdaImm, (bytes >> (i * 8)) as u8);
            self.code.byte(ByteOp::StaDp, 28 + i);
        }
        let again = self.code.label();
        self.code.mark(again);
        self.code.byte(ByteOp::LdaIndirect, 3);
        self.code.byte(ByteOp::StaIndirect, PTR); // long indirect, no DBR dependency
        self.pointer_step(PTR, backward, 1);
        self.pointer_step(3, backward, 1);
        self.pointer_step(28, true, 1);
        self.code.byte(ByteOp::LdaDp, 28);
        self.code.byte(ByteOp::OraDp, 29);
        self.code.byte(ByteOp::OraDp, 30);
        self.code.branch(Branch::NotEqual, again);
    }
    fn compare(
        &mut self,
        dest: TempId,
        bytes: u8,
        signed: bool,
        op: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<(), String> {
        let less = self.code.label();
        let greater = self.code.label();
        let equal = self.code.label();
        let done = self.code.label();
        for i in (0..bytes).rev() {
            self.value_byte(right, i)?;
            if signed && i == bytes - 1 {
                self.code.byte(ByteOp::EorImm, 0x80);
            }
            self.code.byte(ByteOp::StaDp, RIGHT);
            self.value_byte(left, i)?;
            if signed && i == bytes - 1 {
                self.code.byte(ByteOp::EorImm, 0x80);
            }
            self.code.byte(ByteOp::CmpDp, RIGHT);
            self.code.branch(Branch::CarryClear, less);
            self.code.branch(Branch::NotEqual, greater);
        }
        self.code.jump(equal);
        for (label, answer) in [
            (
                less,
                matches!(op, NirCompareOp::Lt | NirCompareOp::Le | NirCompareOp::Ne),
            ),
            (
                greater,
                matches!(op, NirCompareOp::Gt | NirCompareOp::Ge | NirCompareOp::Ne),
            ),
            (
                equal,
                matches!(op, NirCompareOp::Eq | NirCompareOp::Le | NirCompareOp::Ge),
            ),
        ] {
            self.code.mark(label);
            self.code.byte(ByteOp::LdaImm, u8::from(answer));
            self.code.jump(done);
        }
        self.code.mark(done);
        self.save_byte(dest, 0)
    }
    fn call(
        &mut self,
        target: &Mir65816CallTarget,
        args: &[Mir65816Value],
        result: Option<(TempId, ByteSize)>,
        plan: &Mir65816CallPlan,
    ) -> Result<(), String> {
        let padding = outgoing_padding(&plan.arguments, plan.outgoing_bytes)?;
        let direct = match target {
            Mir65816CallTarget::Direct(id) => Some(Target::Routine(RoutineId(*id))),
            Mir65816CallTarget::Helper(id) => Some(Target::Routine(*id)),
            Mir65816CallTarget::Runtime(id) => Some(Target::Runtime(*id)),
            Mir65816CallTarget::Indirect(..) => None,
            Mir65816CallTarget::Builtin(_) => {
                return Err("builtin call requires a resolved native runtime binding".into());
            }
        };
        let outgoing =
            u16::try_from(plan.outgoing_bytes.get()).map_err(|_| "outgoing extent overflow")?;
        let transfer = plan
            .native
            .ok_or("missing native call contract")?
            .transfer
            .peak_bytes()
            .get() as u16;
        self.check_stack(
            outgoing
                .checked_add(transfer)
                .ok_or("call stack overflow")?,
        );
        self.reserve(outgoing);
        assert_eq!(self.code.delta(), u32::from(outgoing));
        self.code.a8();
        self.code.byte(ByteOp::LdaImm, 0);
        for displacement in padding {
            self.code.byte(ByteOp::StaStack, displacement);
        }
        for (value, home) in args.iter().zip(&plan.arguments) {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = home else {
                return Err("invalid outgoing home".into());
            };
            for i in 0..width(*size)? {
                self.value_byte(value, i)?;
                let d = abi::stack::access_displacement(
                    ByteOffset::new(1 + offset.get() + u32::from(i)),
                    ByteSize::ONE,
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
                self.code.byte(ByteOp::StaStack, d.get() as u8);
            }
        }
        if let Some(target) = direct {
            self.code.a16();
            self.code.native_call(target, plan)?; // JSL
        } else {
            let Mir65816CallTarget::Indirect(value, bytes) = target else {
                unreachable!()
            };
            if bytes.get() != 3 || self.value_width(value)? != 3 {
                return Err("indirect call requires a full-width callable".into());
            }
            // Capture the target before pushing: all d,S accesses still use delta=O.
            self.pointer_value(value, PTR)?;
            self.code.a16();
            self.code.byte(ByteOp::LdaDp, PTR + 1);
            self.code.op(Implied::Xba); // XBA; isolate bank without reading a fourth byte
            self.code.word(WordOp::AndImm, 0x00ff);
            self.code.op(Implied::Tay); // TAY
            self.code.byte(ByteOp::LdaDp, PTR);
            self.code.op(Implied::Tax); // TAX
            let resume = self.code.label();
            self.code.op(Implied::Phk); // PHK: real caller bank
            self.code.push_return(resume);
            self.code.op(Implied::Tya); // TYA
            self.code.a8();
            self.code.op(Implied::Pha); // PHA: target bank
            self.code.a16();
            self.code.op(Implied::Txa); // TXA
            self.code.op(Implied::DecA); // DEC A: wrap only the low word, never borrow from bank
            self.code.op(Implied::Pha); // PHA: target PC minus one
            self.code.native_indirect_transfer(plan)?; // RTL: enter callee with ordinary three-byte return frame
            self.code.mark(resume);
        }
        self.release(outgoing, true);
        assert_eq!(self.code.delta(), 0);
        if let Some((id, bytes)) = result {
            let bytes = width(bytes)?;
            if self.temp(id)?.slot().width != bytes {
                return Err("call result width mismatch".into());
            }
            self.code.a8();
            self.save_byte(id, 0)?;
            if bytes > 1 {
                self.code.op(Implied::Xba);
                self.save_byte(id, 1)?;
            } // XBA
            self.code.a16();
            if bytes > 2 {
                self.code.op(Implied::Txa);
                self.code.a8();
                self.save_byte(id, 2)?; // TXA
                if bytes > 3 {
                    self.code.op(Implied::Xba);
                    self.save_byte(id, 3)?;
                }
                self.code.a16();
            }
        }
        Ok(())
    }
    fn byte_constant_return(&mut self, value: &Mir65816Value) -> bool {
        if self.routine.result_home
            != Some(Mir65816AbiHome::NativeResult(
                abi::ResultLocation::A8ZeroExtended,
            ))
        {
            return false;
        }
        let Mir65816Value::U8(value) = value else {
            return false;
        };
        self.code.a16();
        self.code.word(WordOp::LdaImm, u16::from(*value));
        // The full A16 load clears hidden B. Shared teardown preserves A;
        // X is unspecified for BYTE results and needs no preparation.
        true
    }
    fn word_return(&mut self, value: &Mir65816Value) -> Result<bool, String> {
        if self.routine.result_home != Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16))
        {
            return Ok(false);
        }
        // Preflight the complete source before changing code or mode knowledge.
        let Some(operand) = self.word_operand(value)? else {
            return Ok(false);
        };
        self.code.a16();
        self.load_checked_word(operand, Self::word_temp(value));
        // The shared teardown preserves A. X is unspecified for word results.
        Ok(true)
    }
    fn return_value(&mut self, value: Option<&Mir65816Value>) -> Result<(), String> {
        if let Some(value) = value {
            let bytes = match self.routine.result_home {
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A8ZeroExtended)) => 1,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => 2,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
                Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
                _ => return Err("value return has no native result home".into()),
            };
            if !self.byte_constant_return(value) && !self.word_return(value)? {
                self.code.a8();
                self.code.byte(ByteOp::LdaImm, 0);
                for i in 0..4 {
                    self.code.byte(ByteOp::StaDp, RESULT + i);
                }
                for i in 0..bytes {
                    self.value_byte(value, i)?;
                    self.code.byte(ByteOp::StaDp, RESULT + i);
                }
                self.code.a16();
                self.code.byte(ByteOp::LdaDp, RESULT);
                self.code.byte(ByteOp::LdxDp, RESULT + 2); // LDA / LDX
            }
        } else if self.routine.result_home.is_some() {
            return Err("function returns without a value".into());
        }
        self.release(self.frame.extent, value.is_some());
        self.code.native_return(self.routine.result_home)?; // RTL
        Ok(())
    }
}
