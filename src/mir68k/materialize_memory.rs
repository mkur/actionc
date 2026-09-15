//! Memory selection after forming the complete effective address exactly once.
use super::*;

impl Builder<'_> {
    fn native_memory(&self, address: &Mir68kAddress, width: ByteSize) -> bool {
        address.naturally_aligned(width)
            && (self.options.pointer_alignment
                || !matches!(address.base, Mir68kAddressBase::Indirect(_)))
    }

    fn guard_memory(&self, address: &Mir68kAddress, width: ByteSize, volatile: bool) -> bool {
        self.options.guarded_memory
            && !volatile
            && width.get() == 4
            && matches!(
                address.base,
                Mir68kAddressBase::Indirect(
                    Mir68kValue::Temp(..)
                        | Mir68kValue::Param(..)
                        | Mir68kValue::GlobalAddress(..)
                        | Mir68kValue::StaticAddress(..)
                )
            )
    }

    /// A0 already contains base + full-width index * stride + displacement.
    /// D0 is the pending store value. No guest data is read by the check.
    fn memory_guard(&mut self) -> Result<(MachineBlockId, MachineBlockId)> {
        let odd = self.label()?;
        let join = self.label()?;
        self.mov(Width::Long, Ea::A(0), Ea::D(1));
        self.emit(Instruction::AluImmediate {
            operation: Alu::And,
            width: Width::Long,
            value: 1,
            destination: 1,
        });
        self.branch(Condition::NotEqual, odd);
        Ok((odd, join))
    }

    pub(super) fn read_memory(
        &mut self,
        address: &Mir68kAddress,
        width: ByteSize,
        volatile: bool,
    ) -> Result<()> {
        self.address(address, 0)?;
        if self.native_memory(address, width) {
            self.mov(Width::from_bytes(width.get())?, Ea::Indirect(0), Ea::D(0));
        } else if self.guard_memory(address, width, volatile) {
            let (odd, join) = self.memory_guard()?;
            self.mov(Width::Long, Ea::Indirect(0), Ea::D(0));
            self.jump(join);
            self.begin(odd);
            self.read_bytes(width);
            self.begin(join);
        } else {
            self.read_bytes(width);
        }
        Ok(())
    }

    fn read_bytes(&mut self, width: ByteSize) {
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

    pub(super) fn write_memory(
        &mut self,
        address: &Mir68kAddress,
        width: ByteSize,
        volatile: bool,
    ) -> Result<()> {
        self.address(address, 0)?;
        if self.native_memory(address, width) {
            self.mov(Width::from_bytes(width.get())?, Ea::D(0), Ea::Indirect(0));
        } else if self.guard_memory(address, width, volatile) {
            let (odd, join) = self.memory_guard()?;
            self.mov(Width::Long, Ea::D(0), Ea::Indirect(0));
            self.jump(join);
            self.begin(odd);
            self.write_bytes(width);
            self.begin(join);
        } else {
            self.write_bytes(width);
        }
        Ok(())
    }

    fn write_bytes(&mut self, width: ByteSize) {
        for byte in 0..width.get() {
            self.mov(Width::Long, Ea::D(0), Ea::D(1));
            self.shift_immediate(1, false, 8 * (width.get() - byte - 1));
            self.mov(Width::Byte, Ea::D(1), Ea::Displacement(0, byte as i16));
        }
    }
}
