//! Original-MC68000 arithmetic legalization and the bare fault adapter.
use super::*;
use crate::runtime_fault::RuntimeFault;

impl Builder<'_> {
    fn label(&mut self) -> Result<MachineBlockId> {
        let id = MachineBlockId(self.next);
        self.next = self.next.checked_add(1).ok_or("too many machine blocks")?;
        Ok(id)
    }
    fn begin(&mut self, label: MachineBlockId) {
        self.blocks.push(MachineBlock {
            id: self.current,
            instructions: std::mem::take(&mut self.instructions),
        });
        self.current = label;
    }
    fn branch(&mut self, condition: Condition, target: MachineBlockId) {
        self.emit(Instruction::Branch {
            condition,
            target: Address::new(Target::Block(target)),
        });
    }
    fn jump(&mut self, target: MachineBlockId) {
        self.emit(Instruction::Jump(Ea::Absolute(Address::new(
            Target::Block(target),
        ))));
    }
    pub(super) fn fault(&mut self, reason: RuntimeFault) -> Result<()> {
        self.mov(
            Width::Long,
            Ea::Immediate(runtime::fault_code(reason)),
            Ea::D(0),
        );
        self.emit(Instruction::Trap(runtime::FAULT_TRAP));
        // Even an adapter that incorrectly returns cannot resume the operation.
        let guard = self.label()?;
        self.begin(guard);
        self.jump(guard);
        Ok(())
    }
    fn magnitude(&mut self, register: u8, width: Width) -> Result<()> {
        if width == Width::Byte {
            self.emit(Instruction::Extend {
                to: Width::Word,
                register,
            });
        }
        if width != Width::Long {
            self.emit(Instruction::Extend {
                to: Width::Long,
                register,
            });
        }
        self.emit(Instruction::CompareImmediate {
            width: Width::Long,
            value: 0,
            destination: register,
        });
        let ready = self.label()?;
        self.branch(Condition::Plus, ready);
        self.emit(Instruction::Negate {
            width: Width::Long,
            register,
        });
        self.begin(ready);
        Ok(())
    }
    pub(super) fn divide(
        &mut self,
        width: Width,
        signed: bool,
        remainder: bool,
        left: &Mir68kValue,
        right: &Mir68kValue,
    ) -> Result<()> {
        self.emit(Instruction::CompareImmediate {
            width,
            value: 0,
            destination: 1,
        });
        let nonzero = self.label()?;
        self.branch(Condition::NotEqual, nonzero);
        self.fault(RuntimeFault::DivisionByZero)?;
        self.begin(nonzero);
        if signed {
            self.magnitude(0, width)?;
            self.magnitude(1, width)?;
        }

        // Restoring unsigned 32/32 division. Q begins as the dividend; shifting
        // its bits into R also creates room for each quotient bit. After k
        // steps R is bounded by the k-bit input prefix, so 32 bits suffice.
        // D2/D3 are borrowed only inside this call-free sequence and restored.
        self.mov(Width::Long, Ea::D(2), Ea::A(0));
        self.mov(Width::Long, Ea::D(3), Ea::A(1));
        self.mov(Width::Long, Ea::D(1), Ea::D(2)); // divisor
        self.mov(Width::Long, Ea::Immediate(0), Ea::D(1)); // remainder
        self.mov(Width::Long, Ea::Immediate(32), Ea::D(3));
        let head = self.label()?;
        let next = self.label()?;
        self.begin(head);
        self.shift_immediate(0, true, 1);
        self.emit(Instruction::AddExtend {
            width: Width::Long,
            source: 1,
            destination: 1,
        });
        self.emit(Instruction::Alu {
            operation: Alu::Compare,
            width: Width::Long,
            source: 2,
            destination: 1,
        });
        self.branch(Condition::CarrySet, next);
        self.emit(Instruction::Alu {
            operation: Alu::Sub,
            width: Width::Long,
            source: 2,
            destination: 1,
        });
        self.emit(Instruction::AddQuick {
            width: Width::Long,
            delta: 1,
            register: 0,
        });
        self.begin(next);
        self.emit(Instruction::AddQuick {
            width: Width::Long,
            delta: -1,
            register: 3,
        });
        self.branch(Condition::NotEqual, head);
        if remainder {
            self.mov(Width::Long, Ea::D(1), Ea::D(0));
        }
        self.mov(Width::Long, Ea::A(0), Ea::D(2));
        self.mov(Width::Long, Ea::A(1), Ea::D(3));

        if signed {
            // Reuse captured operand signs, keeping the result in frame scratch.
            self.mov(Width::Long, Ea::D(0), Ea::Displacement(6, self.index_slot));
            self.value(left, 0)?;
            if !remainder {
                self.value(right, 1)?;
                self.emit(Instruction::Alu {
                    operation: Alu::Xor,
                    width,
                    source: 1,
                    destination: 0,
                });
            }
            self.emit(Instruction::CompareImmediate {
                width,
                value: 0,
                destination: 0,
            });
            let positive = self.label()?;
            let done = self.label()?;
            self.branch(Condition::Plus, positive);
            self.mov(Width::Long, Ea::Displacement(6, self.index_slot), Ea::D(0));
            self.emit(Instruction::Negate { width, register: 0 });
            self.jump(done);
            self.begin(positive);
            self.mov(Width::Long, Ea::Displacement(6, self.index_slot), Ea::D(0));
            self.begin(done);
        }
        Ok(())
    }
}
