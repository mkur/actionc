//! Conservative stack homes and instruction selection from MIR68K alone.
use super::{machine::*, *};
use std::collections::{BTreeMap, BTreeSet};

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
            Mir68kTerminator::Exit => {
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
        frame.spill_bytes = ByteSize::new(cursor - start);
        cursor = cursor
            .checked_add(frame.outgoing.size.get())
            .ok_or("outgoing area overflow")?;
        displacement(-(cursor as i64))?;
        frame.extent = ByteSize::new(cursor);
        frame.outgoing.frame_offset = -(cursor as i32);
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
        if !self.routine.prologue.parameter_copies.is_empty() {
            return Err("parameter frame copies are not implemented yet".into());
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
            Mir68kValue::Param(_) => return Err("parameter values are not implemented yet".into()),
        };
        if width != Width::Long {
            self.mov(Width::Long, Ea::Immediate(0), Ea::D(register));
        }
        self.mov(width, source, Ea::D(register));
        Ok(())
    }
    fn address(&mut self, address: &Mir68kAddress) -> Result<Ea> {
        if address.index.is_some() {
            return Err("indexed addresses are not implemented yet".into());
        }
        let offset = i64::from(address.displacement.get());
        let ea = match address.base {
            Mir68kAddressBase::Static(NirStorageId::Global(id)) => Ea::Absolute(Address {
                target: Target::Data(Mir68kDataId::Global(id)),
                addend: offset,
            }),
            Mir68kAddressBase::Static(NirStorageId::Local(id)) => Ea::Absolute(Address {
                target: Target::Data(Mir68kDataId::Local(self.routine.id, id)),
                addend: offset,
            }),
            Mir68kAddressBase::AutomaticFrame(id) => {
                let object = self
                    .frame
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .ok_or("missing automatic object")?;
                Ea::Displacement(6, displacement(i64::from(object.frame_offset) + offset)?)
            }
            Mir68kAddressBase::External(Mir68kExternalAddress::Absolute(a)) => {
                Ea::Absolute(Address {
                    target: Target::Absolute(
                        u32::try_from(a.value).map_err(|_| "absolute address exceeds 32 bits")?,
                    ),
                    addend: offset,
                })
            }
            _ => return Err("address form is not implemented yet".into()),
        };
        Ok(ea)
    }
    fn op(&mut self, op: &Mir68kOp) -> Result<()> {
        match op {
            Mir68kOp::Store {
                address,
                value,
                width,
                ..
            } => {
                aligned(address, *width)?;
                self.value(value, 0)?;
                let destination = self.address(address)?;
                self.mov(Width::from_bytes(width.get())?, Ea::D(0), destination);
            }
            Mir68kOp::Load {
                dest,
                width,
                address,
                ..
            } => {
                aligned(address, *width)?;
                let source = self.address(address)?;
                self.mov(Width::from_bytes(width.get())?, source, Ea::D(0));
                self.save(*dest, *width)?;
            }
            Mir68kOp::AddressOf {
                dest,
                width,
                address,
            } => {
                let source = self.address(address)?;
                self.emit(Instruction::Lea {
                    source,
                    destination: 0,
                });
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
                ..
            } => {
                self.value(left, 0)?;
                self.value(right, 1)?;
                let machine_width = Width::from_bytes(width.get())?;
                match operation {
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
            _ => {
                return Err(format!(
                    "native instruction selection does not yet support {op:?}"
                ));
            }
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
            _ => return Err("unresolved native terminator".into()),
        }
        Ok(())
    }
}

fn displacement(value: i64) -> Result<i16> {
    i16::try_from(value)
        .map_err(|_| "native frame exceeds original MC68000 signed-16 displacement limits".into())
}
fn aligned(address: &Mir68kAddress, width: ByteSize) -> Result<()> {
    if width.get() > 1
        && (address.base_alignment.is_none_or(|a| a.get() < 2)
            || address.displacement.get() & 1 != 0)
    {
        Err("unaligned memory access is not implemented yet".into())
    } else {
        Ok(())
    }
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
