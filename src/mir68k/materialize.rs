//! Conservative stack homes and instruction selection from MIR68K alone.
use super::{machine::*, *};
use std::collections::{BTreeMap, BTreeSet};

#[path = "materialize_arithmetic.rs"]
mod arithmetic;

type Result<T> = std::result::Result<T, String>;

pub fn materialize(program: &Mir68kProgram) -> Result<MachineProgram> {
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
        if routine.entry.external {
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
        let mut builder = Builder::new(routine, &mut next)?;
        let reachable = reachable_blocks(routine)?;
        for block in &routine.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }
            builder.instructions.clear();
            builder.current = builder.labels[&block.id];
            if block.id == routine.blocks[0].id {
                builder.prologue()?;
            }
            for op in &block.ops {
                builder
                    .op(op)
                    .map_err(|e| format!("{} / {:?}: {e}", routine.name, block.id))?;
            }
            builder
                .terminator(&block.terminator)
                .map_err(|e| format!("{} / {:?}: {e}", routine.name, block.id))?;
            machine.blocks.append(&mut builder.blocks);
            machine.blocks.push(MachineBlock {
                id: builder.current,
                instructions: std::mem::take(&mut builder.instructions),
            });
        }
        next = builder.next;
        machine.routines.push(MachineRoutine {
            id: routine.id,
            entry: builder.labels[&routine.blocks[0].id],
            frame: builder.frame,
        });
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
    routine: &'a Mir68kRoutine,
    frame: Mir68kFramePlan,
    temps: BTreeMap<TempId, i16>,
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
}

impl<'a> Builder<'a> {
    fn new(routine: &'a Mir68kRoutine, next: &mut u32) -> Result<Self> {
        let mut frame = routine.frame.clone();
        let mut cursor = frame
            .automatic_bytes
            .get()
            .checked_add(1)
            .ok_or("frame overflow")?
            & !1;
        let start = cursor;
        let mut temps = BTreeMap::new();
        for (id, ty) in &routine.temps {
            Width::from_bytes(ty.width.ok_or("temporary has no width")?.get())?;
            cursor = cursor.checked_add(4).ok_or("temporary area overflow")?;
            temps.insert(*id, displacement(-(cursor as i64))?);
        }
        let edge_count = routine
            .blocks
            .iter()
            .map(|b| b.params.len())
            .max()
            .unwrap_or(0);
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
            routine,
            frame,
            temps,
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
        })
    }
    fn emit(&mut self, op: Instruction) {
        self.instructions.push(op);
    }
    fn mov(&mut self, width: Width, source: Ea, destination: Ea) {
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
        Ok(Ea::Displacement(
            6,
            *self.temps.get(&id).ok_or("temporary has no stack home")?,
        ))
    }
    fn save(&mut self, id: TempId, width: ByteSize) -> Result<()> {
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
            self.emit(Instruction::AddAddress {
                source: Ea::Immediate(address.displacement.get()),
                destination: register,
            });
        }
        if let Some(index) = &address.index {
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
    fn read_memory(&mut self, address: &Mir68kAddress, width: ByteSize) -> Result<()> {
        self.address(address, 0)?;
        if naturally_aligned(address, width) {
            self.mov(Width::from_bytes(width.get())?, Ea::Indirect(0), Ea::D(0));
        } else {
            self.mov(Width::Long, Ea::Immediate(0), Ea::D(0));
            self.mov(Width::Long, Ea::Immediate(0), Ea::D(1));
            for byte in 0..width.get() {
                if byte != 0 {
                    self.shift_immediate(0, true, 8);
                }
                self.mov(Width::Byte, Ea::Displacement(0, byte as i16), Ea::D(1));
                self.emit(Instruction::Alu {
                    operation: Alu::Or,
                    width: Width::Long,
                    source: 1,
                    destination: 0,
                });
            }
        }
        Ok(())
    }
    fn write_memory(&mut self, address: &Mir68kAddress, width: ByteSize) -> Result<()> {
        self.address(address, 0)?;
        if naturally_aligned(address, width) {
            self.mov(Width::from_bytes(width.get())?, Ea::D(0), Ea::Indirect(0));
        } else {
            for byte in 0..width.get() {
                self.mov(Width::Long, Ea::D(0), Ea::D(1));
                self.shift_immediate(1, false, 8 * (width.get() - byte - 1));
                self.mov(Width::Byte, Ea::D(1), Ea::Displacement(0, byte as i16));
            }
        }
        Ok(())
    }
    fn op(&mut self, op: &Mir68kOp) -> Result<()> {
        match op {
            Mir68kOp::Fault(reason) => self.fault(*reason)?,
            Mir68kOp::Store {
                address,
                value,
                width,
                ..
            } => {
                self.value(value, 0)?;
                self.write_memory(address, *width)?;
            }
            Mir68kOp::Load {
                dest,
                width,
                address,
                ..
            } => {
                self.read_memory(address, *width)?;
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
                self.value(left, 0)?;
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
                self.value(left, 0)?;
                self.value(right, 1)?;
                self.emit(Instruction::Alu {
                    operation: Alu::Compare,
                    width: Width::from_bytes(width.get())?,
                    source: 1,
                    destination: 0,
                });
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
    fn edge(&mut self, edge: &Mir68kEdge) -> Result<()> {
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

    fn terminator(&mut self, term: &Mir68kTerminator) -> Result<()> {
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
                self.emit(Instruction::Unlink(6));
                self.emit(Instruction::Rts);
            }
            Mir68kTerminator::Goto(edge) => self.edge(edge)?,
            Mir68kTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                self.value(condition, 0)?;
                let alternate = MachineBlockId(self.next);
                self.next = self.next.checked_add(1).ok_or("too many machine blocks")?;
                self.emit(Instruction::Branch {
                    condition: Condition::Equal,
                    target: Address::new(Target::Block(alternate)),
                });
                self.edge(then_edge)?;
                self.blocks.push(MachineBlock {
                    id: self.current,
                    instructions: std::mem::take(&mut self.instructions),
                });
                self.current = alternate;
                self.edge(else_edge)?;
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
fn naturally_aligned(address: &Mir68kAddress, width: ByteSize) -> bool {
    width.get() == 1
        || address.base_alignment.is_some_and(|a| a.get() >= 2)
            && address.displacement.get() & 1 == 0
            && address
                .index
                .as_ref()
                .is_none_or(|i| i.stride.get() & 1 == 0)
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
