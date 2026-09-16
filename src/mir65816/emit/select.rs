use super::{allocation::width, *};
use std::collections::BTreeMap;

// ABI call-clobbered domain scratch. Nothing here survives a call.
const PTR: u8 = abi::generated::DP_POINTER0_OFFSET as u8;
const RESULT: u8 = 8;
const RIGHT: u8 = 16;
const INDEX: u8 = 20;

#[derive(Clone, Copy)]
enum Memory {
    Stack(u32),
    Absolute(u32),
    Symbol(Target, u32),
    /// Displacement retained for [PTR],Y instead of modifying the far pointer.
    Pointer(u16),
}

struct Builder<'a> {
    routine: &'a Mir65816Routine,
    frame: AllocatedFrame,
    code: Code,
    blocks: BTreeMap<BlockId, Label>,
    /// Current downward S movement relative to the allocated body frame.
    delta: u32,
}

pub(super) fn routine(routine: &Mir65816Routine) -> Result<MachineRoutine, String> {
    let mut b = Builder {
        routine,
        frame: AllocatedFrame::new(routine)?,
        code: Code::default(),
        blocks: BTreeMap::new(),
        delta: 0,
    };
    if routine.blocks.is_empty() || !routine.blocks[0].params.is_empty() {
        return Err("routine requires an entry block without edge parameters".into());
    }
    for block in &routine.blocks {
        if b.blocks.insert(block.id, b.code.label()).is_some() {
            return Err("duplicate block identity".into());
        }
    }
    b.check_stack(b.frame.extent);
    b.code.op(0x1b); // TCS: checked new S in A, no write/push before the check.
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
    for (index, block) in routine.blocks.iter().enumerate() {
        b.code.mark(b.blocks[&block.id]);
        for op in &block.ops {
            b.operation(op)
                .map_err(|e| format!("b{}: {e}", block.id.0))?;
        }
        b.code.a16(); // Every MIR control-flow boundary has the ABI width.
        match &block.terminator {
            Mir65816Terminator::Goto(edge) => b.edge(edge)?,
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
                b.code.branch(0xd0, yes); // BNE
                b.edge(else_edge)?;
                b.code.mark(yes);
                b.edge(then_edge)?;
            }
            Mir65816Terminator::Return { value, .. } => b.return_value(value.as_ref())?,
            Mir65816Terminator::Fallthrough => {
                let next = routine
                    .blocks
                    .get(index + 1)
                    .ok_or("unresolved terminal fallthrough")?;
                b.edge(&Mir65816Edge {
                    target: next.id,
                    args: vec![],
                })?;
            }
            Mir65816Terminator::Exit => {
                return Err("terminal exit requires a native runtime adapter".into());
            }
        }
    }
    Ok(MachineRoutine {
        id: routine.id,
        frame: b.frame,
        code: b.code,
    })
}

impl Builder<'_> {
    fn incoming(&self, id: ParamId) -> Result<u32, String> {
        let param = self
            .routine
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("unknown parameter")?;
        let Mir65816AbiHome::StackArgument { offset, size, .. } = param.incoming else {
            return Err("invalid parameter home".into());
        };
        abi::stack::incoming_displacement(ByteSize::new(self.frame.extent.into()), offset, size)
            .map(|d| d.get())
            .map_err(|e| e.to_string())
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
        let param = self
            .routine
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("unknown parameter")?;
        let Mir65816AbiHome::StackArgument { size, .. } = param.incoming else {
            return Err("invalid parameter home".into());
        };
        Ok((
            if let Some(object) = param.frame_object {
                self.object(object)?
            } else {
                self.incoming(id)?
            },
            width(size)?,
        ))
    }
    fn temp(&self, id: TempId) -> Result<Slot, String> {
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
            ByteSize::new(self.delta),
        )
        .map(|d| d.get() as u8)
        .map_err(|e| e.to_string())
    }
    fn load_memory(&mut self, memory: Memory, byte: u32) -> Result<(), String> {
        self.memory(0xa3, 0xaf, 0xb7, memory, byte)
    }
    fn store_memory(&mut self, memory: Memory, byte: u32) -> Result<(), String> {
        self.memory(0x83, 0x8f, 0x97, memory, byte)
    }
    fn memory(
        &mut self,
        stack: u8,
        long: u8,
        indirect: u8,
        memory: Memory,
        byte: u32,
    ) -> Result<(), String> {
        match memory {
            Memory::Stack(offset) => self.code.byte(stack, self.displacement(offset, byte)?),
            Memory::Absolute(address) => self
                .code
                .long(long, address.checked_add(byte).ok_or("address overflow")?)?,
            Memory::Symbol(target, offset) => self.code.reference(
                long,
                target,
                offset.checked_add(byte).ok_or("address offset overflow")?,
                None,
            ),
            Memory::Pointer(offset) => {
                self.code.word(
                    0xa0,
                    u16::try_from(u32::from(offset) + byte)
                        .map_err(|_| "indirect displacement exceeds Y")?,
                ); // LDY
                self.code.byte(indirect, PTR); // LDA/STA [PTR],Y: linear 24-bit access
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
        // Private frame transfers can stay in A16 for a three-byte value by
        // overlapping the two words. Never touch a fourth byte, and never
        // duplicate a read/write through an external or indirect address.
        if wide
            && bytes == 3
            && let (Memory::Stack(src), Memory::Stack(dst)) = (source, destination)
            && (src == dst || src.abs_diff(dst) >= 3)
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
                if slot.width != width(*bytes)? {
                    return Err("temporary width mismatch".into());
                }
                Some(Memory::Stack(slot.offset.into()))
            }
            Mir65816Value::Param(id) => Some(Memory::Stack(self.parameter(*id)?.0)),
            _ => None,
        })
    }
    fn value_width(&self, value: &Mir65816Value) -> Result<u8, String> {
        Ok(match value {
            Mir65816Value::U8(_) => 1,
            Mir65816Value::U16(_) => 2,
            Mir65816Value::U24(_) => 3,
            Mir65816Value::U32(_) => 4,
            Mir65816Value::Param(id) => self.parameter(*id)?.1,
            Mir65816Value::Null(w)
            | Mir65816Value::Address(_, w)
            | Mir65816Value::StaticAddress(_, w)
            | Mir65816Value::Temp(_, w)
            | Mir65816Value::GlobalAddress(_, w)
            | Mir65816Value::RoutineAddress(_, w) => width(*w)?,
        })
    }
    /// A8 byte load, leaving carry intact for multi-byte arithmetic.
    fn value_byte(&mut self, value: &Mir65816Value, byte: u8) -> Result<(), String> {
        // NIR permits narrow operands (notably loop-step constants). Values are
        // zero-extended here; signed widening is an explicit Cast operation.
        if byte >= self.value_width(value)? {
            self.code.byte(0xa9, 0);
            return Ok(());
        }
        match value {
            Mir65816Value::U8(v) => self.code.byte(0xa9, *v),
            Mir65816Value::U16(v) => self.code.byte(0xa9, (*v >> (byte * 8)) as u8),
            Mir65816Value::U24(v) | Mir65816Value::U32(v) => {
                self.code.byte(0xa9, (*v >> (byte * 8)) as u8)
            }
            Mir65816Value::Null(_) => self.code.byte(0xa9, 0),
            Mir65816Value::Address(v, _) => self.code.byte(0xa9, (v.value >> (byte * 8)) as u8),
            Mir65816Value::Temp(id, w) => {
                let slot = self.temp(*id)?;
                if slot.width != width(*w)? {
                    return Err("temporary width mismatch".into());
                }
                self.load_memory(Memory::Stack(slot.offset.into()), byte.into())?;
            }
            Mir65816Value::Param(id) => {
                self.load_memory(Memory::Stack(self.parameter(*id)?.0), byte.into())?
            }
            Mir65816Value::StaticAddress(id, _) => self.code.reference(
                0xa9,
                Target::Data(Mir65816DataId::Static(*id)),
                0,
                Some(byte),
            ),
            Mir65816Value::GlobalAddress(id, _) => self.code.reference(
                0xa9,
                Target::Data(Mir65816DataId::Global(*id)),
                0,
                Some(byte),
            ),
            Mir65816Value::RoutineAddress(id, _) => {
                self.code
                    .reference(0xa9, Target::Routine(RoutineId(*id)), 0, Some(byte))
            }
        }
        Ok(())
    }
    fn save_byte(&mut self, dest: TempId, byte: u8) -> Result<(), String> {
        let slot = self.temp(dest)?;
        if byte >= slot.width {
            return Err("temporary write exceeds its width".into());
        }
        self.store_memory(Memory::Stack(slot.offset.into()), byte.into())
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
                self.code.byte(0x85, scratch);
                if bytes >= 3 {
                    // Both source and scratch have three owned bytes. Reading
                    // the overlapping word avoids a bank-byte mode switch.
                    self.load_memory(memory, 1)?;
                    self.code.byte(0x85, scratch + 1);
                    return Ok(());
                }
                self.code.a8();
                self.value_byte(value, 2)?;
                self.code.byte(0x85, scratch + 2);
                return Ok(());
            }
        }
        self.code.a8();
        for i in 0..3 {
            if i < bytes {
                self.value_byte(value, i)?;
            } else {
                self.code.byte(0xa9, 0);
            }
            self.code.byte(0x85, scratch + i);
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
                self.pointer_value(value, PTR)?;
                Memory::Pointer(0)
            }
        };
        if address.index.is_none() && !matches!(memory, Memory::Pointer(_)) {
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
                Memory::Pointer(_) => unreachable!(),
            });
        }
        if !matches!(memory, Memory::Pointer(_)) {
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
                    self.code.op(0x18);
                    for i in 0..3 {
                        self.code.byte(0xa5, PTR + i);
                        self.code.byte(0x65, INDEX + i);
                        self.code.byte(0x85, PTR + i);
                    }
                }
                self.code.byte(0x06, INDEX); // ASL / ROL, low byte first
                self.code.byte(0x26, INDEX + 1);
                self.code.byte(0x26, INDEX + 2);
            }
        }
        if displacement >= 0x1000000 {
            return Err("pointer displacement exceeds 24 bits".into());
        }
        // Leave room for every byte of the largest scalar (four bytes).
        // AddressOf and aggregate copies explicitly materialize this offset.
        if displacement <= u32::from(u16::MAX) - 3 {
            return Ok(Memory::Pointer(displacement as u16));
        }
        if displacement != 0 {
            self.code.a8();
            self.code.op(0x18);
            for i in 0..3 {
                self.code.byte(0xa5, PTR + i);
                self.code.byte(0x69, (displacement >> (i * 8)) as u8);
                self.code.byte(0x85, PTR + i);
            }
        }
        Ok(Memory::Pointer(0))
    }
    fn address_to_pointer(&mut self, memory: Memory) -> Result<(), String> {
        self.code.a8();
        match memory {
            Memory::Stack(offset) => {
                let displacement = self.displacement(offset, 0)?;
                self.code.a16();
                self.code.op(0x3b);
                self.code.op(0x18); // TSC / CLC
                self.code.word(0x69, displacement.into());
                self.code.byte(0x85, PTR);
                self.code.a8();
                self.code.byte(0xa9, 0);
                self.code.byte(0x85, PTR + 2);
            }
            Memory::Absolute(a) => {
                if a >= 0x1000000 {
                    return Err("24-bit address overflow".into());
                }
                for i in 0..3 {
                    self.code.byte(0xa9, (a >> (i * 8)) as u8);
                    self.code.byte(0x85, PTR + i);
                }
            }
            Memory::Symbol(t, offset) => {
                for i in 0..3 {
                    self.code.reference(0xa9, t, offset, Some(i));
                    self.code.byte(0x85, PTR + i);
                }
            }
            Memory::Pointer(offset) => {
                if offset != 0 {
                    self.pointer_step(PTR, false, offset.into());
                }
            }
        }
        Ok(())
    }
    fn check_stack(&mut self, bytes: u16) {
        // A/X/Y are caller-clobbered. X retains the unchanged S for the raw
        // overflow adapter. Neither branch changes I, D, DBR or the stack.
        let within = self.code.label();
        let fault = self.code.label();
        let done = self.code.label();
        self.code.op(0x3b);
        self.code.op(0xaa); // TSC / TAX
        self.code
            .byte(0xc5, abi::generated::DP_STACK_CEILING_OFFSET as u8);
        self.code.branch(0x90, within);
        self.code.branch(0xf0, within);
        self.code.jump(fault);
        self.code.mark(within);
        self.code.op(0x38);
        self.code.word(0xe9, bytes); // SEC / SBC
        self.code.branch(0x90, fault);
        self.code
            .byte(0xc5, abi::generated::DP_STACK_FLOOR_OFFSET as u8);
        self.code.branch(0xb0, done);
        self.code.mark(fault);
        self.code.word(0xa9, bytes);
        self.code.reference(0x5c, Target::StackOverflow, 0, None);
        self.code.mark(done);
    }
    fn reserve(&mut self, bytes: u16) {
        self.code.op(0x3b);
        self.code.op(0x38);
        self.code.word(0xe9, bytes);
        self.code.op(0x1b);
    }
    fn release(&mut self, bytes: u16, preserve_result: bool) {
        if bytes != 0 {
            // TAY; TSC; CLC; ADC #bytes; TCS; TYA. Preserve the entire A/X result.
            if preserve_result {
                self.code.op(0xa8);
            }
            self.code.op(0x3b);
            self.code.op(0x18);
            self.code.word(0x69, bytes);
            self.code.op(0x1b);
            if preserve_result {
                self.code.op(0x98);
            }
        }
    }
    fn edge(&mut self, edge: &Mir65816Edge) -> Result<(), String> {
        let block = self
            .routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .ok_or("unknown branch target")?;
        if edge.args.len() != block.params.len() {
            return Err("edge argument count mismatch".into());
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
        self.code.jump(self.blocks[&edge.target]);
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
            self.code.a16();
            return self.call(target, args, *result, plan);
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
                if slot.width != width(*bytes)? {
                    return Err("load temporary width mismatch".into());
                }
                self.transfer(
                    memory,
                    Memory::Stack(slot.offset.into()),
                    slot.width,
                    !volatile,
                )?;
            }
            Mir65816Op::Store {
                address,
                value,
                width: bytes,
                volatile,
            } => {
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
                    self.code.byte(0xa5, PTR + i);
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
                    self.code.op(0x38);
                }
                for i in 0..width(*bytes)? {
                    self.value_byte(value, i)?;
                    if *operation == NirUnaryOp::Neg {
                        self.code.byte(0x85, RIGHT);
                        self.code.byte(0xa9, 0);
                        self.code.byte(0xe5, RIGHT);
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
                        self.code.branch(0x10, positive); // BPL
                        self.code.byte(0xa9, 0xff);
                        self.code.jump(ready);
                        self.code.mark(positive);
                        self.code.byte(0xa9, 0);
                        self.code.mark(ready);
                    } else {
                        self.code.byte(0xa9, 0);
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
                self.code.op(if *subtract { 0x38 } else { 0x18 });
                for i in 0..width(*bytes)? {
                    if *offset_signed && i >= offset_bytes {
                        let positive = self.code.label();
                        let ready = self.code.label();
                        self.value_byte(offset, offset_bytes - 1)?;
                        self.code.branch(0x10, positive);
                        self.code.byte(0xa9, 0xff);
                        self.code.jump(ready);
                        self.code.mark(positive);
                        self.code.byte(0xa9, 0);
                        self.code.mark(ready);
                    } else {
                        self.value_byte(offset, i)?;
                    }
                    self.code.byte(0x85, RIGHT);
                    self.value_byte(base, i)?;
                    self.code.byte(if *subtract { 0xe5 } else { 0x65 }, RIGHT);
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
                self.code.op(0x18);
                0x65
            }
            NirBinaryOp::Sub => {
                self.code.op(0x38);
                0xe5
            }
            NirBinaryOp::And => 0x25,
            NirBinaryOp::Or => 0x05,
            NirBinaryOp::Xor => 0x45,
            _ => {
                return Err(format!(
                    "native emission does not support integer {operation:?}"
                ));
            }
        };
        for i in 0..bytes {
            self.value_byte(right, i)?;
            self.code.byte(0x85, RIGHT);
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
            self.code.byte(0x85, RESULT + i);
        }
        for i in 1..self.value_width(count)? {
            self.value_byte(count, i)?;
            self.code.branch(0xd0, zero);
        }
        self.value_byte(count, 0)?;
        self.code.byte(0xc9, bytes * 8);
        self.code.branch(0xb0, zero);
        self.code.byte(0xc9, 0);
        self.code.branch(0xf0, ready);
        self.code.a16();
        self.code.word(0x29, 0xff);
        self.code.op(0xaa);
        self.code.a8();
        self.code.mark(loop_start);
        if left_shift {
            self.code.byte(0x06, RESULT);
            for i in 1..bytes {
                self.code.byte(0x26, RESULT + i);
            }
        } else {
            self.code.byte(0x46, RESULT + bytes - 1);
            for i in (0..bytes - 1).rev() {
                self.code.byte(0x66, RESULT + i);
            }
        }
        self.code.op(0xca);
        self.code.branch(0xd0, loop_start);
        self.code.jump(ready);
        self.code.mark(zero);
        self.code.byte(0xa9, 0);
        for i in 0..bytes {
            self.code.byte(0x85, RESULT + i);
        }
        self.code.mark(ready);
        for i in 0..bytes {
            self.code.byte(0xa5, RESULT + i);
            self.save_byte(dest, i)?;
        }
        Ok(())
    }
    fn pointer_step(&mut self, pointer: u8, subtract: bool, amount: u32) {
        self.code.op(if subtract { 0x38 } else { 0x18 });
        for i in 0..3 {
            self.code.byte(0xa5, pointer + i);
            self.code.byte(
                if subtract { 0xe9 } else { 0x69 },
                (amount >> (i * 8)) as u8,
            );
            self.code.byte(0x85, pointer + i);
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
            self.code.byte(0xa5, PTR + i);
            self.code.byte(0x85, 24 + i);
        }
        let memory = self.prepare_address(source)?;
        self.address_to_pointer(memory)?;
        for i in 0..3 {
            self.code.byte(0xa5, PTR + i);
            self.code.byte(0x85, 3 + i);
            self.code.byte(0xa5, 24 + i);
            self.code.byte(0x85, PTR + i);
        }
        let forward = self.code.label();
        let backward = self.code.label();
        let done = self.code.label();
        if overlap_safe {
            for i in (0..3).rev() {
                self.code.byte(0xa5, PTR + i);
                self.code.byte(0xc5, 3 + i);
                self.code.branch(0x90, forward);
                self.code.branch(0xd0, backward);
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
            self.code.byte(0xa9, (bytes >> (i * 8)) as u8);
            self.code.byte(0x85, 28 + i);
        }
        let again = self.code.label();
        self.code.mark(again);
        self.code.byte(0xa7, 3);
        self.code.byte(0x87, PTR); // long indirect, no DBR dependency
        self.pointer_step(PTR, backward, 1);
        self.pointer_step(3, backward, 1);
        self.pointer_step(28, true, 1);
        self.code.byte(0xa5, 28);
        self.code.byte(0x05, 29);
        self.code.byte(0x05, 30);
        self.code.branch(0xd0, again);
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
                self.code.byte(0x49, 0x80);
            }
            self.code.byte(0x85, RIGHT);
            self.value_byte(left, i)?;
            if signed && i == bytes - 1 {
                self.code.byte(0x49, 0x80);
            }
            self.code.byte(0xc5, RIGHT);
            self.code.branch(0x90, less);
            self.code.branch(0xd0, greater);
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
            self.code.byte(0xa9, u8::from(answer));
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
        let direct = match target {
            Mir65816CallTarget::Direct(id) => Some(Target::Routine(RoutineId(*id))),
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
        self.delta = outgoing.into();
        self.code.a8();
        self.code.byte(0xa9, 0);
        for i in 1..=outgoing {
            self.code.byte(
                0x83,
                u8::try_from(i).map_err(|_| "outgoing displacement overflow")?,
            );
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
                self.code.byte(0x83, d.get() as u8);
            }
        }
        if let Some(target) = direct {
            self.code.a16();
            self.code.reference(0x22, target, 0, None); // JSL
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
            self.code.byte(0xa5, PTR + 1);
            self.code.op(0xeb); // XBA; isolate bank without reading a fourth byte
            self.code.word(0x29, 0x00ff);
            self.code.op(0xa8); // TAY
            self.code.byte(0xa5, PTR);
            self.code.op(0xaa); // TAX
            let resume = self.code.label();
            self.code.op(0x4b); // PHK: real caller bank
            self.code.push_return(resume);
            self.code.op(0x98); // TYA
            self.code.a8();
            self.code.op(0x48); // PHA: target bank
            self.code.a16();
            self.code.op(0x8a); // TXA
            self.code.op(0x3a); // DEC A: wrap only the low word, never borrow from bank
            self.code.op(0x48); // PHA: target PC minus one
            self.code.op(0x6b); // RTL: enter callee with ordinary three-byte return frame
            self.code.mark(resume);
        }
        self.release(outgoing, true);
        self.delta = 0;
        if let Some((id, bytes)) = result {
            let bytes = width(bytes)?;
            if self.temp(id)?.width != bytes {
                return Err("call result width mismatch".into());
            }
            self.code.a8();
            self.save_byte(id, 0)?;
            if bytes > 1 {
                self.code.op(0xeb);
                self.save_byte(id, 1)?;
            } // XBA
            self.code.a16();
            if bytes > 2 {
                self.code.op(0x8a);
                self.code.a8();
                self.save_byte(id, 2)?; // TXA
                if bytes > 3 {
                    self.code.op(0xeb);
                    self.save_byte(id, 3)?;
                }
                self.code.a16();
            }
        }
        Ok(())
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
            self.code.a8();
            self.code.byte(0xa9, 0);
            for i in 0..4 {
                self.code.byte(0x85, RESULT + i);
            }
            for i in 0..bytes {
                self.value_byte(value, i)?;
                self.code.byte(0x85, RESULT + i);
            }
            self.code.a16();
            self.code.byte(0xa5, RESULT);
            self.code.byte(0xa6, RESULT + 2); // LDA / LDX
        } else if self.routine.result_home.is_some() {
            return Err("function returns without a value".into());
        }
        self.release(self.frame.extent, value.is_some());
        self.code.op(0x6b); // RTL
        Ok(())
    }
}
