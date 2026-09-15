//! Verified register/stack homes and instruction selection from MIR68K alone.
use super::{machine::*, *};
use std::collections::{BTreeMap, BTreeSet};

#[path = "materialize_arithmetic.rs"]
mod arithmetic;
#[path = "materialize_memory.rs"]
mod memory;
#[path = "materialize_selection.rs"]
mod selection;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub forward_temporaries: bool,
    pub select_instructions: bool,
    pub relax_branches: bool,
    pub pointer_alignment: bool,
    /// Guard unknown nonvolatile indirect longword accesses at runtime.
    pub guarded_memory: bool,
    pub control_flow: bool,
    pub register_allocation: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            forward_temporaries: true,
            select_instructions: true,
            relax_branches: true,
            pointer_alignment: true,
            guarded_memory: true,
            control_flow: true,
            register_allocation: true,
        }
    }
}
impl Options {
    pub const fn conservative() -> Self {
        Self {
            forward_temporaries: false,
            select_instructions: false,
            relax_branches: false,
            pointer_alignment: false,
            guarded_memory: false,
            control_flow: false,
            register_allocation: false,
        }
    }
}

pub fn materialize(program: &Mir68kProgram) -> Result<MachineProgram> {
    materialize_with_options(program, Options::default())
}

pub fn materialize_with_options(
    program: &Mir68kProgram,
    options: Options,
) -> Result<MachineProgram> {
    materialize_program(program, options, None)
}

pub(super) fn materialize_program(
    program: &Mir68kProgram,
    options: Options,
    amiga: Option<&BTreeMap<RoutineId, super::amiga::ConsoleService>>,
) -> Result<MachineProgram> {
    verify::verify_contract(program).map_err(|e| format!("invalid MIR68K: {e:?}"))?;
    let entry = program
        .routines
        .iter()
        .find(|r| Some(r.id) == program.entry)
        .ok_or("native execution requires a program entry")?;
    if !entry.params.is_empty() || entry.signature.result.is_some() {
        return Err("native program entry must be a parameterless PROC".into());
    }
    let mut machine = MachineProgram::default();
    let mut next = 0;
    for routine in &program.routines {
        let first_block = machine.blocks.len();
        if routine.entry.external {
            if let Some(service) = amiga.and_then(|bindings| bindings.get(&routine.id)) {
                super::amiga::validate_console_signature(*service, &routine.signature)?;
                if !matches!(routine.entry.placement, crate::nir::NirRoutinePlacement::Relocatable) {
                    return Err("Amiga console entry must be relocatable".into());
                }
                let entry = MachineBlockId(next);
                next = next.checked_add(1).ok_or("too many machine blocks")?;
                machine.blocks.push(MachineBlock { id: entry, instructions: vec![Instruction::Jump(Ea::Absolute(Address::new(Target::PlatformRoutine(super::amiga::PlatformRoutineId::Console(*service)))))] });
                machine.routines.push(MachineRoutine { id: routine.id, entry, frame: routine.frame.clone() });
                continue;
            }
            return Err(format!(
                "{}: external native routine entry requires an adapter",
                routine.name
            ));
        }
        if !matches!(
            routine.entry.placement,
            crate::nir::NirRoutinePlacement::Relocatable
        ) {
            return Err(format!(
                "{}: fixed native routine placement is unsupported",
                routine.name
            ));
        }
        let mut builder = Builder::new(routine, &mut next, options)?;
        builder.amiga = amiga.is_some();
        let reachable = reachable_blocks(routine)?;
        let uses = super::analysis::use_counts(routine);
        for block in &routine.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }
            builder.instructions.clear();
            builder.current = builder.labels[&block.id];
            if block.id == routine.blocks[0].id {
                builder.prologue()?;
            }
            let compare = options
                .control_flow
                .then(|| super::analysis::branch_compare(block, &uses))
                .flatten();
            for op in block
                .ops
                .iter()
                .take(block.ops.len() - usize::from(compare.is_some()))
            {
                builder
                    .op(op)
                    .map_err(|e| format!("{} / {:?}: {e}", routine.name, block.id))?;
            }
            builder
                .terminator(&block.terminator, compare)
                .map_err(|e| format!("{} / {:?}: {e}", routine.name, block.id))?;
            machine.blocks.append(&mut builder.blocks);
            machine.blocks.push(MachineBlock {
                id: builder.current,
                instructions: std::mem::take(&mut builder.instructions),
            });
        }
        if options.forward_temporaries {
            super::temporary_forwarding::forward(
                &mut machine.blocks[first_block..],
                &builder.temps.values().copied().collect(),
            );
        }
        if options.control_flow {
            super::control_flow::fallthrough(&mut machine.blocks[first_block..]);
        }
        next = builder.next;
        machine.routines.push(MachineRoutine {
            id: routine.id,
            entry: builder.labels[&routine.blocks[0].id],
            frame: builder.frame,
        });
    }
    if options.relax_branches {
        super::branch_relaxation::relax(&mut machine)?;
    }
    Ok(machine)
}

pub fn reachable_blocks(routine: &Mir68kRoutine) -> Result<BTreeSet<BlockId>> {
    let mut pending = vec![
        routine
            .blocks
            .first()
            .ok_or("native routine has no entry block")?
            .id,
    ];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let block = routine
            .blocks
            .iter()
            .find(|b| b.id == id)
            .ok_or("missing block")?;
        match &block.terminator {
            Mir68kTerminator::Goto(e) => pending.push(e.target),
            Mir68kTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => pending.extend([then_edge.target, else_edge.target]),
            Mir68kTerminator::Fallthrough => {
                return Err(format!(
                    "{}: reachable unresolved fallthrough",
                    routine.name
                ));
            }
            Mir68kTerminator::Exit if !matches!(block.ops.last(), Some(Mir68kOp::Fault(_))) => {
                return Err(format!(
                    "{}: native terminal exit requires a runtime adapter",
                    routine.name
                ));
            }
            _ => {}
        }
    }
    Ok(visited)
}

struct Builder<'a> {
    amiga: bool,
    routine: &'a Mir68kRoutine,
    frame: Mir68kFramePlan,
    temps: BTreeMap<TempId, i16>,
    allocation: super::allocation::Allocation,
    saved_registers: Vec<(u8, i16)>,
    labels: BTreeMap<BlockId, MachineBlockId>,
    instructions: Vec<Instruction>,
    blocks: Vec<MachineBlock>,
    current: MachineBlockId,
    next: u32,
    edge_slots: Vec<i16>,
    index_slot: i16,
    callee_slot: i16,
    call_slots: Vec<i16>,
    copy_slot: i16,
    options: Options,
}

struct EdgeCopy {
    dest: TempId,
    width: ByteSize,
    destination: Ea,
    // None denotes the normalized longword saved to break a cycle.
    source: Option<Mir68kValue>,
    source_home: Option<Ea>,
}

impl<'a> Builder<'a> {
    fn new(routine: &'a Mir68kRoutine, next: &mut u32, options: Options) -> Result<Self> {
        let mut frame = routine.frame.clone();
        let mut cursor = frame
            .automatic_bytes
            .get()
            .checked_add(1)
            .ok_or("frame overflow")?
            & !1;
        let allocation = if options.register_allocation {
            super::allocation::allocate(routine, options.control_flow)?
        } else {
            super::allocation::Allocation::default()
        };
        let mut saved_registers = Vec::new();
        for register in allocation
            .registers
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
        {
            saved_registers.push((register, reserve(&mut cursor, 4)?));
        }
        frame.saved_register_bytes = ByteSize::new(saved_registers.len() as u32 * 4);
        let start = cursor;
        let mut temps = BTreeMap::new();
        for (id, ty) in &routine.temps {
            Width::from_bytes(ty.width.ok_or("temporary has no width")?.get())?;
            if allocation.registers.contains_key(id) || allocation.discarded.contains(id) {
                continue;
            }
            cursor = cursor.checked_add(4).ok_or("temporary area overflow")?;
            temps.insert(*id, displacement(-(cursor as i64))?);
        }
        let edge_count = routine
            .blocks
            .iter()
            .map(|b| b.params.len())
            .max()
            .unwrap_or(0);
        let edge_count = if options.register_allocation {
            edge_count.min(1)
        } else {
            edge_count
        };
        let mut edge_slots = Vec::new();
        for _ in 0..edge_count {
            cursor = cursor.checked_add(4).ok_or("edge copy area overflow")?;
            edge_slots.push(displacement(-i64::from(cursor))?);
        }
        let index_slot = reserve(&mut cursor, 4)?;
        let callee_slot = reserve(&mut cursor, 4)?;
        let argument_count = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match op {
                Mir68kOp::Call { args, .. } => Some(args.len()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut call_slots = Vec::new();
        for _ in 0..argument_count {
            call_slots.push(reserve(&mut cursor, 4)?);
        }
        let copy_bytes = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match op {
                Mir68kOp::Copy { bytes, .. } => Some(bytes.get()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let copy_slot = reserve(&mut cursor, copy_bytes)?;
        frame.spill_bytes = ByteSize::new(cursor - start);
        cursor = cursor
            .checked_add(frame.outgoing.size.get())
            .ok_or("outgoing area overflow")?;
        displacement(-(cursor as i64))?;
        frame.extent = ByteSize::new(cursor);
        frame.outgoing.frame_offset = -(cursor as i32);
        let mut ranges: Vec<(i64, i64)> = frame
            .objects
            .iter()
            .filter(|o| !o.size.is_zero())
            .map(|o| {
                (
                    i64::from(o.frame_offset),
                    i64::from(o.frame_offset) + i64::from(o.size.get()),
                )
            })
            .collect();
        ranges.extend(
            saved_registers
                .iter()
                .map(|(_, offset)| (i64::from(*offset), i64::from(*offset) + 4)),
        );
        ranges.extend(
            temps
                .values()
                .map(|offset| (i64::from(*offset), i64::from(*offset) + 4)),
        );
        ranges.extend(
            edge_slots
                .iter()
                .chain(&call_slots)
                .chain([&index_slot, &callee_slot])
                .map(|offset| (i64::from(*offset), i64::from(*offset) + 4)),
        );
        if copy_bytes != 0 {
            ranges.push((
                i64::from(copy_slot),
                i64::from(copy_slot) + i64::from(copy_bytes),
            ));
        }
        if !frame.outgoing.size.is_zero() {
            ranges.push((
                -i64::from(cursor),
                -i64::from(cursor) + i64::from(frame.outgoing.size.get()),
            ));
        }
        ranges.sort_unstable();
        if ranges
            .iter()
            .any(|(start, end)| *start < -i64::from(cursor) || *end > 0)
            || ranges.windows(2).any(|r| r[0].1 > r[1].0)
        {
            return Err("overlapping or out-of-frame native homes".into());
        }
        for parameter in &frame.parameters {
            let Mir68kAbiHome::StackArgument { offset, size } = parameter.incoming else {
                return Err("native parameter needs a stack argument home".into());
            };
            displacement(8 + i64::from(offset.get()))?;
            if 8 + u64::from(offset.get()) + u64::from(size.get()) > 32768 {
                return Err(
                    "native incoming argument exceeds signed-16 displacement limits".into(),
                );
            }
        }
        let mut labels = BTreeMap::new();
        for block in &routine.blocks {
            labels.insert(block.id, MachineBlockId(*next));
            *next = next.checked_add(1).ok_or("too many machine blocks")?;
        }
        Ok(Self {
            amiga: false,
            routine,
            frame,
            temps,
            allocation,
            saved_registers,
            labels,
            instructions: Vec::new(),
            blocks: Vec::new(),
            current: MachineBlockId(0),
            next: *next,
            edge_slots,
            index_slot,
            callee_slot,
            call_slots,
            copy_slot,
            options,
        })
    }
    fn emit(&mut self, op: Instruction) {
        self.instructions.push(op);
    }
    fn mov(&mut self, width: Width, source: Ea, destination: Ea) {
        if self.options.select_instructions
            && width == Width::Long
            && let (Ea::Immediate(value), Ea::D(register)) = (source, destination)
            && let Ok(value) = i8::try_from(value as i32)
        {
            self.emit(Instruction::MoveQuick { value, register });
            return;
        }
        self.emit(Instruction::Move {
            width,
            source,
            destination,
        });
    }
    fn prologue(&mut self) -> Result<()> {
        self.emit(Instruction::Link {
            register: 6,
            displacement: displacement(-i64::from(self.frame.extent.get()))?,
        });
        for (register, offset) in self.saved_registers.clone() {
            self.mov(Width::Long, Ea::D(register), Ea::Displacement(6, offset));
        }
        for copy in &self.routine.prologue.parameter_copies {
            let Mir68kAbiHome::StackArgument { offset, size } = copy.source else {
                return Err("parameter copy requires a stack argument".into());
            };
            let destination = self
                .frame
                .objects
                .iter()
                .find(|o| o.id == copy.destination)
                .ok_or("missing copied parameter home")?
                .frame_offset;
            self.mov(
                Width::from_bytes(size.get())?,
                Ea::Displacement(6, displacement(8 + i64::from(offset.get()))?),
                Ea::Displacement(6, displacement(i64::from(destination))?),
            );
        }
        Ok(())
    }
    fn temp(&self, id: TempId) -> Result<Ea> {
        if let Some(register) = self.allocation.registers.get(&id) {
            return Ok(Ea::D(*register));
        }
        Ok(Ea::Displacement(
            6,
            *self.temps.get(&id).ok_or("temporary has no stack home")?,
        ))
    }
    fn save(&mut self, id: TempId, width: ByteSize) -> Result<()> {
        if self.allocation.discarded.contains(&id) {
            return Ok(());
        }
        self.mov(Width::from_bytes(width.get())?, Ea::D(0), self.temp(id)?);
        Ok(())
    }
    fn value(&mut self, value: &Mir68kValue, register: u8) -> Result<()> {
        let width = Width::from_bytes(
            verify::value_width(value, self.routine)
                .ok_or("value has no width")?
                .get(),
        )?;
        let source = match value {
            Mir68kValue::U8(v) => Ea::Immediate(*v as u32),
            Mir68kValue::U16(v) => Ea::Immediate(*v as u32),
            Mir68kValue::U32(v) => Ea::Immediate(*v),
            Mir68kValue::Null(_) => Ea::Immediate(0),
            Mir68kValue::Address(a, _) => {
                Ea::Immediate(u32::try_from(a.value).map_err(|_| "address value exceeds 32 bits")?)
            }
            Mir68kValue::Temp(id, _) => self.temp(*id)?,
            Mir68kValue::StaticAddress(id, _) => {
                Ea::ImmediateAddress(Address::new(Target::Data(Mir68kDataId::Static(*id))))
            }
            Mir68kValue::GlobalAddress(id, _) => {
                Ea::ImmediateAddress(Address::new(Target::Data(Mir68kDataId::Global(*id))))
            }
            Mir68kValue::RoutineAddress(id, _) => {
                Ea::ImmediateAddress(Address::new(Target::Routine(*id)))
            }
            Mir68kValue::Param(id) => self.parameter(*id)?,
        };
        if width != Width::Long {
            self.mov(Width::Long, Ea::Immediate(0), Ea::D(register));
        }
        self.mov(width, source, Ea::D(register));
        Ok(())
    }
    fn parameter(&self, id: ParamId) -> Result<Ea> {
        let parameter = self
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("parameter has no home")?;
        let offset = if let Some(id) = parameter.frame_object {
            i64::from(
                self.frame
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .ok_or("missing parameter frame object")?
                    .frame_offset,
            )
        } else {
            let Mir68kAbiHome::StackArgument { offset, .. } = parameter.incoming else {
                return Err("parameter is not stack passed".into());
            };
            8 + i64::from(offset.get())
        };
        Ok(Ea::Displacement(6, displacement(offset)?))
    }
    /// Compute into An, using only D1 and the private index staging slot.
    /// D0 may hold a store value; the other address scratch may hold copy source.
    fn address(&mut self, address: &Mir68kAddress, register: u8) -> Result<()> {
        let base = match &address.base {
            Mir68kAddressBase::Static(NirStorageId::Global(id))
            | Mir68kAddressBase::External(Mir68kExternalAddress::Global(id)) => {
                Ea::Absolute(Address::new(Target::Data(Mir68kDataId::Global(*id))))
            }
            Mir68kAddressBase::Static(NirStorageId::Local(id)) => Ea::Absolute(Address::new(
                Target::Data(Mir68kDataId::Local(self.routine.id, *id)),
            )),
            Mir68kAddressBase::AutomaticFrame(id) => {
                let object = self
                    .frame
                    .objects
                    .iter()
                    .find(|o| o.id == *id)
                    .ok_or("missing automatic object")?;
                Ea::Displacement(6, displacement(i64::from(object.frame_offset))?)
            }
            Mir68kAddressBase::Parameter(id) => self.parameter(*id)?,
            Mir68kAddressBase::External(Mir68kExternalAddress::Absolute(a)) => {
                Ea::Absolute(Address::absolute(
                    u32::try_from(a.value).map_err(|_| "absolute address exceeds 32 bits")?,
                ))
            }
            Mir68kAddressBase::Indirect(value) => {
                self.value(value, 1)?;
                self.mov(Width::Long, Ea::D(1), Ea::A(register));
                Ea::Indirect(register)
            }
            _ => return Err("invalid address base".into()),
        };
        if base != Ea::Indirect(register) {
            self.emit(Instruction::Lea {
                source: base,
                destination: register,
            });
        }
        if address.displacement.get() != 0 {
            if self.options.select_instructions {
                self.add_displacement(register, address.displacement.get());
            } else {
                self.emit(Instruction::AddAddress {
                    source: Ea::Immediate(address.displacement.get()),
                    destination: register,
                });
            }
        }
        if let Some(index) = &address.index {
            if self.options.select_instructions {
                if let Some(value) = selection::constant(&index.value) {
                    self.add_displacement(register, value.wrapping_mul(index.stride.get()));
                    return Ok(());
                }
                if index.stride.get().is_power_of_two() {
                    self.value(&index.value, 1)?;
                    self.shift_immediate(1, true, index.stride.get().trailing_zeros());
                    self.emit(Instruction::AddAddress {
                        source: Ea::D(1),
                        destination: register,
                    });
                    return Ok(());
                }
            }
            self.value(&index.value, 1)?;
            self.mov(Width::Long, Ea::D(1), Ea::Displacement(6, self.index_slot));
            // Full-width constant scaling, including non-power-of-two record
            // strides. Original 68000 indexed modes cannot do this scaling.
            for bit in 0..32 {
                if index.stride.get() & (1u32 << bit) == 0 {
                    continue;
                }
                self.mov(Width::Long, Ea::Displacement(6, self.index_slot), Ea::D(1));
                self.shift_immediate(1, true, bit);
                self.emit(Instruction::AddAddress {
                    source: Ea::D(1),
                    destination: register,
                });
            }
        }
        Ok(())
    }
    fn shift_immediate(&mut self, register: u8, left: bool, mut count: u32) {
        while count != 0 {
            let step = count.min(8);
            self.emit(Instruction::LogicalShift {
                width: Width::Long,
                left,
                count: ShiftCount::Immediate(step as u8),
                register,
            });
            count -= step;
        }
    }
    fn op(&mut self, op: &Mir68kOp) -> Result<()> {
        match op {
            Mir68kOp::Fault(reason) => self.fault(*reason)?,
            Mir68kOp::Store {
                address,
                value,
                width,
                volatile,
                ..
            } => {
                self.value(value, 0)?;
                self.write_memory(address, *width, *volatile)?;
            }
            Mir68kOp::Load {
                dest,
                width,
                address,
                volatile,
                ..
            } => {
                self.read_memory(address, *width, *volatile)?;
                self.save(*dest, *width)?;
            }
            Mir68kOp::AddressOf {
                dest,
                width,
                address,
            } => {
                self.address(address, 0)?;
                self.mov(Width::Long, Ea::A(0), Ea::D(0));
                self.save(*dest, *width)?;
            }
            Mir68kOp::Cast {
                dest,
                from,
                from_signed,
                to,
                value,
                ..
            } => {
                self.value(value, 0)?;
                if to > from && *from_signed {
                    if from.get() == 1 {
                        self.emit(Instruction::Extend {
                            to: Width::Word,
                            register: 0,
                        });
                    }
                    if to.get() == 4 {
                        self.emit(Instruction::Extend {
                            to: Width::Long,
                            register: 0,
                        });
                    }
                }
                self.save(*dest, *to)?;
            }
            Mir68kOp::Unary {
                dest,
                width,
                operation,
                value,
            } => {
                self.value(value, 0)?;
                if *operation == NirUnaryOp::Neg {
                    self.emit(Instruction::Negate {
                        width: Width::from_bytes(width.get())?,
                        register: 0,
                    });
                }
                self.save(*dest, *width)?;
            }
            Mir68kOp::Binary {
                dest,
                width,
                operation,
                left,
                right,
                signed,
            } => {
                // Both operands are captured MIR values: commuting multiplication
                // here cannot reorder source evaluation or memory reads.
                let (left, right) = if self.options.select_instructions
                    && *operation == NirBinaryOp::Mul
                    && selection::constant(left).is_some()
                    && selection::constant(right).is_none()
                {
                    (right, left)
                } else {
                    (left, right)
                };
                self.value(left, 0)?;
                if self.options.select_instructions
                    && self.constant_binary(Width::from_bytes(width.get())?, *operation, right)
                {
                    self.save(*dest, *width)?;
                    return Ok(());
                }
                self.value(right, 1)?;
                let machine_width = Width::from_bytes(width.get())?;
                match operation {
                    NirBinaryOp::Mul => self.multiply(machine_width, left)?,
                    NirBinaryOp::Div | NirBinaryOp::Mod => self.divide(
                        machine_width,
                        *signed,
                        *operation == NirBinaryOp::Mod,
                        left,
                        right,
                    )?,
                    NirBinaryOp::Lsh | NirBinaryOp::Rsh => {
                        self.emit(Instruction::LogicalShift {
                            width: machine_width,
                            left: *operation == NirBinaryOp::Lsh,
                            count: ShiftCount::Register(1),
                            register: 0,
                        });
                        // MC68000 masks the count modulo 64. Mask that provisional
                        // result to zero when the full Action count >= bit width.
                        self.emit(Instruction::CompareImmediate {
                            width: Width::Long,
                            value: width.get() * 8,
                            destination: 1,
                        });
                        self.emit(Instruction::SetCondition {
                            condition: Condition::CarrySet,
                            register: 1,
                        });
                        self.emit(Instruction::Extend {
                            to: Width::Word,
                            register: 1,
                        });
                        self.emit(Instruction::Extend {
                            to: Width::Long,
                            register: 1,
                        });
                        self.emit(Instruction::Alu {
                            operation: Alu::And,
                            width: Width::Long,
                            source: 1,
                            destination: 0,
                        });
                    }
                    operation => {
                        let operation = match operation {
                            NirBinaryOp::Add => Alu::Add,
                            NirBinaryOp::Sub => Alu::Sub,
                            NirBinaryOp::And => Alu::And,
                            NirBinaryOp::Or => Alu::Or,
                            NirBinaryOp::Xor => Alu::Xor,
                            _ => {
                                return Err(format!(
                                    "native {operation:?} requires a helper not supported by this emitter"
                                ));
                            }
                        };
                        self.emit(Instruction::Alu {
                            operation,
                            width: machine_width,
                            source: 1,
                            destination: 0,
                        });
                    }
                }
                self.save(*dest, *width)?;
            }
            Mir68kOp::Compare {
                dest,
                width,
                signed,
                operation,
                left,
                right,
            } => {
                self.compare_flags(left, right, *width)?;
                self.emit(Instruction::SetCondition {
                    condition: compare_condition(*operation, *signed),
                    register: 0,
                });
                self.mov(Width::Long, Ea::Immediate(1), Ea::D(1));
                self.emit(Instruction::Alu {
                    operation: Alu::And,
                    width: Width::Long,
                    source: 1,
                    destination: 0,
                });
                self.save(*dest, ByteSize::ONE)?;
            }
            Mir68kOp::PointerOffset {
                dest,
                width,
                base,
                offset,
                subtract,
            } => {
                self.value(base, 0)?;
                self.value(offset, 1)?;
                self.emit(Instruction::Alu {
                    operation: if *subtract { Alu::Sub } else { Alu::Add },
                    width: Width::Long,
                    source: 1,
                    destination: 0,
                });
                self.save(*dest, *width)?;
            }
            Mir68kOp::Copy {
                destination,
                source,
                bytes,
                ..
            } => {
                self.address(source, 0)?;
                self.address(destination, 1)?;
                for byte in 0..bytes.get() {
                    self.mov(
                        Width::Byte,
                        Ea::Displacement(0, displacement(i64::from(byte))?),
                        Ea::Displacement(
                            6,
                            displacement(i64::from(self.copy_slot) + i64::from(byte))?,
                        ),
                    );
                }
                for byte in 0..bytes.get() {
                    self.mov(
                        Width::Byte,
                        Ea::Displacement(
                            6,
                            displacement(i64::from(self.copy_slot) + i64::from(byte))?,
                        ),
                        Ea::Displacement(1, displacement(i64::from(byte))?),
                    );
                }
            }
            Mir68kOp::Call {
                target,
                args,
                result,
                plan,
                ..
            } => {
                match target {
                    Mir68kCallTarget::Direct(_) | Mir68kCallTarget::Indirect(_, _) => {}
                    _ => {
                        return Err(
                            "native external/runtime calls require an explicit adapter".into()
                        );
                    }
                }
                for (index, arg) in args.iter().enumerate() {
                    self.value(arg, 0)?;
                    self.mov(
                        Width::Long,
                        Ea::D(0),
                        Ea::Displacement(6, self.call_slots[index]),
                    );
                }
                if let Mir68kCallTarget::Indirect(value, _) = target {
                    self.value(value, 0)?;
                    self.mov(Width::Long, Ea::D(0), Ea::Displacement(6, self.callee_slot));
                }
                for (index, home) in plan.arguments.iter().enumerate() {
                    let Mir68kAbiHome::StackArgument { offset, size } = home else {
                        return Err("native call arguments require stack homes".into());
                    };
                    self.mov(
                        Width::Long,
                        Ea::Displacement(6, self.call_slots[index]),
                        Ea::D(0),
                    );
                    // BYTE occupies the first byte of its even-sized slot.
                    self.mov(
                        Width::from_bytes(size.get())?,
                        Ea::D(0),
                        Ea::Displacement(7, displacement(i64::from(offset.get()))?),
                    );
                }
                match target {
                    Mir68kCallTarget::Direct(id) => self.emit(Instruction::Jsr(Ea::Absolute(
                        Address::new(Target::Routine(*id)),
                    ))),
                    Mir68kCallTarget::Indirect(_, _) => {
                        self.mov(Width::Long, Ea::Displacement(6, self.callee_slot), Ea::A(0));
                        self.emit(Instruction::Jsr(Ea::Indirect(0)));
                    }
                    _ => unreachable!(),
                }
                if let Some((dest, width)) = result {
                    if matches!(plan.result, Some(Mir68kAbiHome::AddressRegister(0))) {
                        self.mov(Width::Long, Ea::A(0), Ea::D(0));
                    }
                    self.save(*dest, *width)?;
                }
            }
        }
        Ok(())
    }
    fn multiply(&mut self, width: Width, left: &Mir68kValue) -> Result<()> {
        // Truncation makes signed and unsigned products identical at the
        // resolved result width. MULU.W suffices for the low byte/word.
        self.emit(Instruction::MultiplyUnsignedWord {
            source: 1,
            destination: 0,
        });
        if width == Width::Long {
            // a*b = alo*blo + ((ahi*blo + alo*bhi) << 16), modulo 2^32.
            // Reload only captured NIR values, never the source expression.
            self.mov(Width::Long, Ea::D(0), Ea::A(0));
            self.value(left, 0)?;
            self.shift_immediate(0, false, 16);
            self.emit(Instruction::MultiplyUnsignedWord {
                source: 1,
                destination: 0,
            });
            self.mov(Width::Long, Ea::D(0), Ea::A(1));
            self.value(left, 0)?;
            self.shift_immediate(1, false, 16);
            self.emit(Instruction::MultiplyUnsignedWord {
                source: 1,
                destination: 0,
            });
            self.mov(Width::Long, Ea::A(1), Ea::D(1));
            self.emit(Instruction::Alu {
                operation: Alu::Add,
                width: Width::Long,
                source: 1,
                destination: 0,
            });
            self.shift_immediate(0, true, 16);
            self.mov(Width::Long, Ea::A(0), Ea::D(1));
            self.emit(Instruction::Alu {
                operation: Alu::Add,
                width: Width::Long,
                source: 1,
                destination: 0,
            });
        }
        Ok(())
    }
    fn compare_flags(
        &mut self,
        left: &Mir68kValue,
        right: &Mir68kValue,
        width: ByteSize,
    ) -> Result<()> {
        self.value(left, 0)?;
        if self.options.select_instructions
            && let Some(value) = selection::constant(right)
        {
            self.emit(Instruction::CompareImmediate {
                width: Width::from_bytes(width.get())?,
                value,
                destination: 0,
            });
        } else {
            self.value(right, 1)?;
            self.emit(Instruction::Alu {
                operation: Alu::Compare,
                width: Width::from_bytes(width.get())?,
                source: 1,
                destination: 0,
            });
        }
        Ok(())
    }

    fn branch_edges(
        &mut self,
        condition: Condition,
        then_edge: &Mir68kEdge,
        else_edge: &Mir68kEdge,
    ) -> Result<()> {
        if self.options.control_flow && then_edge.args.is_empty() {
            self.emit(Instruction::Branch {
                condition,
                target: Address::new(Target::Block(self.labels[&then_edge.target])),
            });
            return self.edge(else_edge);
        }
        if self.options.control_flow && else_edge.args.is_empty() {
            self.emit(Instruction::Branch {
                condition: condition.inverse(),
                target: Address::new(Target::Block(self.labels[&else_edge.target])),
            });
            return self.edge(then_edge);
        }
        let alternate = MachineBlockId(self.next);
        self.next = self.next.checked_add(1).ok_or("too many machine blocks")?;
        self.emit(Instruction::Branch {
            condition: condition.inverse(),
            target: Address::new(Target::Block(alternate)),
        });
        self.edge(then_edge)?;
        self.blocks.push(MachineBlock {
            id: self.current,
            instructions: std::mem::take(&mut self.instructions),
        });
        self.current = alternate;
        self.edge(else_edge)
    }

    fn edge(&mut self, edge: &Mir68kEdge) -> Result<()> {
        if self.options.register_allocation {
            self.parallel_edge(edge)?;
            self.emit(Instruction::Jump(Ea::Absolute(Address::new(
                Target::Block(self.labels[&edge.target]),
            ))));
            return Ok(());
        }
        // Stage every source before writing any destination. This also handles
        // cyclic transfers on a loop backedge without overwriting a live source.
        for (index, value) in edge.args.iter().enumerate() {
            self.value(value, 0)?;
            self.mov(
                Width::Long,
                Ea::D(0),
                Ea::Displacement(6, self.edge_slots[index]),
            );
        }
        let params = &self
            .routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .ok_or("missing edge target")?
            .params;
        for (index, (dest, ty)) in params.iter().enumerate() {
            self.mov(
                Width::Long,
                Ea::Displacement(6, self.edge_slots[index]),
                Ea::D(0),
            );
            self.save(*dest, ty.width.ok_or("block parameter has no width")?)?;
        }
        self.emit(Instruction::Jump(Ea::Absolute(Address::new(
            Target::Block(self.labels[&edge.target]),
        ))));
        Ok(())
    }

    fn parallel_edge(&mut self, edge: &Mir68kEdge) -> Result<()> {
        // Sources are either typed values or a normalized longword in the
        // cycle slot. A narrow big-endian stack home is never read as a long.
        let params = &self
            .routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .ok_or("missing edge target")?
            .params;
        let mut pending = Vec::new();
        for ((dest, ty), source) in params.iter().zip(&edge.args) {
            if self.allocation.discarded.contains(dest) {
                continue;
            }
            let destination = self.temp(*dest)?;
            let source_home = match source {
                Mir68kValue::Temp(id, _) => Some(self.temp(*id)?),
                _ => None,
            };
            if source_home != Some(destination) {
                pending.push(EdgeCopy {
                    dest: *dest,
                    width: ty.width.ok_or("block parameter has no width")?,
                    destination,
                    source: Some(source.clone()),
                    source_home,
                });
            }
        }
        while !pending.is_empty() {
            if let Some(index) = pending.iter().position(|copy| {
                !pending
                    .iter()
                    .any(|other| other.source_home == Some(copy.destination))
            }) {
                let copy = pending.remove(index);
                if let Some(source) = copy.source {
                    self.value(&source, 0)?;
                } else {
                    self.mov(
                        Width::Long,
                        Ea::Displacement(6, self.edge_slots[0]),
                        Ea::D(0),
                    );
                }
                self.save(copy.dest, copy.width)?;
            } else {
                let home = pending[0].destination;
                let reader = pending
                    .iter()
                    .find(|copy| copy.source_home == Some(home))
                    .and_then(|copy| copy.source.as_ref())
                    .ok_or("invalid edge copy cycle")?;
                self.value(reader, 0)?;
                self.mov(
                    Width::Long,
                    Ea::D(0),
                    Ea::Displacement(6, self.edge_slots[0]),
                );
                for copy in &mut pending {
                    if copy.source_home == Some(home) {
                        copy.source = None;
                        copy.source_home = None;
                    }
                }
            }
        }
        Ok(())
    }

    fn terminator(&mut self, term: &Mir68kTerminator, compare: Option<&Mir68kOp>) -> Result<()> {
        match term {
            Mir68kTerminator::Return { value, .. } => {
                if let Some(value) = value {
                    self.value(value, 0)?;
                    if matches!(
                        self.routine.result_home,
                        Some(Mir68kAbiHome::AddressRegister(0))
                    ) {
                        self.mov(Width::Long, Ea::D(0), Ea::A(0));
                    }
                }
                for (register, offset) in self.saved_registers.clone() {
                    self.mov(Width::Long, Ea::Displacement(6, offset), Ea::D(register));
                }
                self.emit(Instruction::Unlink(6));
                self.emit(Instruction::Rts);
            }
            Mir68kTerminator::Goto(edge) => self.edge(edge)?,
            Mir68kTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let condition = if let Some(Mir68kOp::Compare {
                    width,
                    signed,
                    operation,
                    left,
                    right,
                    ..
                }) = compare
                {
                    self.compare_flags(left, right, *width)?;
                    compare_condition(*operation, *signed)
                } else {
                    self.value(condition, 0)?;
                    Condition::NotEqual
                };
                self.branch_edges(condition, then_edge, else_edge)?;
            }
            Mir68kTerminator::Exit => {} // A verified final Fault emitted its non-returning guard.
            _ => return Err("unresolved native terminator".into()),
        }
        Ok(())
    }
}

fn displacement(value: i64) -> Result<i16> {
    i16::try_from(value)
        .map_err(|_| "native frame exceeds original MC68000 signed-16 displacement limits".into())
}
fn reserve(cursor: &mut u32, size: u32) -> Result<i16> {
    *cursor = cursor
        .checked_add(size)
        .and_then(|n| n.checked_add(1))
        .ok_or("frame reservation overflow")?
        & !1;
    displacement(-i64::from(*cursor))
}

fn compare_condition(operation: NirCompareOp, signed: bool) -> Condition {
    match (operation, signed) {
        (NirCompareOp::Eq, _) => Condition::Equal,
        (NirCompareOp::Ne, _) => Condition::NotEqual,
        (NirCompareOp::Lt, true) => Condition::Less,
        (NirCompareOp::Le, true) => Condition::LessOrEqual,
        (NirCompareOp::Gt, true) => Condition::Greater,
        (NirCompareOp::Ge, true) => Condition::GreaterOrEqual,
        (NirCompareOp::Lt, false) => Condition::CarrySet,
        (NirCompareOp::Le, false) => Condition::LowOrSame,
        (NirCompareOp::Gt, false) => Condition::High,
        (NirCompareOp::Ge, false) => Condition::CarryClear,
    }
}
