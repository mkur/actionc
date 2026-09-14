//! Small integer/count-string console routines; no optimizer or HUNK decisions.
use super::*;

impl Builder {
    pub(super) fn console(&mut self, service: ConsoleService) -> Result<(), String> {
        use ConsoleService::*;
        if matches!(
            service,
            PrintB | PrintBE | PrintC | PrintCE | PrintI | PrintIE
        ) {
            return self.decimal(service);
        }
        let mut code = vec![Instruction::Link {
            register: 6,
            displacement: -16,
        }];
        match service {
            Print | PrintE => code.extend([
                mov(Ea::Displacement(6, 8), Ea::A(0)),
                Instruction::MoveQuick {
                    value: 0,
                    register: 0,
                },
                byte(Ea::Indirect(0), Ea::D(0)),
                Instruction::AddAddress {
                    source: Ea::Immediate(1),
                    destination: 0,
                },
                mov(Ea::A(0), slot(0)),
                mov(Ea::D(0), slot(4)),
                call(PlatformRoutineId::WriteSpan),
            ]),
            Put => code.extend([
                Instruction::Lea {
                    source: Ea::Displacement(6, 8),
                    destination: 0,
                },
                mov(Ea::A(0), slot(0)),
                mov(Ea::Immediate(1), slot(4)),
                call(PlatformRoutineId::WriteSpan),
            ]),
            PutE => {}
            _ => unreachable!(),
        }
        if matches!(service, PutE | PrintE) {
            code.extend([
                byte(Ea::Immediate(10), Ea::Displacement(6, -4)),
                Instruction::Lea {
                    source: Ea::Displacement(6, -4),
                    destination: 0,
                },
                mov(Ea::A(0), slot(0)),
                mov(Ea::Immediate(1), slot(4)),
                call(PlatformRoutineId::WriteSpan),
            ]);
        }
        code.extend([Instruction::Unlink(6), Instruction::Rts]);
        self.routine(PlatformRoutineId::Console(service), code)
    }

    fn decimal(&mut self, service: ConsoleService) -> Result<(), String> {
        use ConsoleService::*;
        let width = if matches!(service, PrintB | PrintBE) {
            Width::Byte
        } else {
            Width::Word
        };
        let unsigned = self.label()?;
        let mut code = vec![
            Instruction::Link {
                register: 6,
                displacement: -32,
            },
            Instruction::MoveQuick {
                value: 0,
                register: 0,
            },
            Instruction::Move {
                width,
                source: Ea::Displacement(6, 8),
                destination: Ea::D(0),
            },
            Instruction::Lea {
                source: Ea::Displacement(6, -16),
                destination: 0,
            },
            byte(Ea::Immediate(0), Ea::Displacement(6, -8)),
        ];
        if matches!(service, PrintI | PrintIE) {
            code.extend([
                // Widen before negation: INT minimum becomes magnitude 32768.
                Instruction::Extend {
                    to: Width::Long,
                    register: 0,
                },
                cmp(0, 0),
                local_branch(Condition::Plus, unsigned),
                Instruction::Negate {
                    width: Width::Long,
                    register: 0,
                },
                byte(Ea::Immediate(u32::from(b'-')), Ea::Indirect(0)),
                Instruction::AddAddress {
                    source: Ea::Immediate(1),
                    destination: 0,
                },
            ]);
        }
        self.routine(PlatformRoutineId::Console(service), code)?;
        self.block(unsigned, vec![]);
        // Five bounded decimal places, at most 45 subtractions. Only scratch
        // registers are used; the result buffer fits sign + five digits + LF.
        for place in [10000, 1000, 100, 10, 1] {
            let init = self.label()?;
            let again = self.label()?;
            let digit = self.label()?;
            let emit = self.label()?;
            let next = self.label()?;
            self.block(
                init,
                vec![Instruction::MoveQuick {
                    value: b'0' as i8,
                    register: 1,
                }],
            );
            self.block(
                again,
                vec![
                    cmp(place, 0),
                    local_branch(Condition::CarrySet, digit),
                    Instruction::AluImmediate {
                        operation: Alu::Sub,
                        width: Width::Long,
                        value: place,
                        destination: 0,
                    },
                    Instruction::AddQuick {
                        width: Width::Long,
                        delta: 1,
                        register: 1,
                    },
                    Instruction::Jump(Ea::Absolute(Address::new(Target::Block(again)))),
                ],
            );
            let mut code = vec![];
            if place != 1 {
                code.extend([
                    cmp(u32::from(b'0'), 1),
                    local_branch(Condition::NotEqual, emit),
                    byte(Ea::Displacement(6, -8), Ea::D(1)),
                    local_branch(Condition::Equal, next),
                    Instruction::MoveQuick {
                        value: b'0' as i8,
                        register: 1,
                    },
                ]);
            }
            self.block(digit, code);
            self.block(
                emit,
                vec![
                    byte(Ea::D(1), Ea::Indirect(0)),
                    Instruction::AddAddress {
                        source: Ea::Immediate(1),
                        destination: 0,
                    },
                    byte(Ea::Immediate(1), Ea::Displacement(6, -8)),
                ],
            );
            self.block(next, vec![]);
        }
        let end = self.label()?;
        let mut code = vec![];
        if matches!(service, PrintBE | PrintCE | PrintIE) {
            code.extend([
                byte(Ea::Immediate(10), Ea::Indirect(0)),
                Instruction::AddAddress {
                    source: Ea::Immediate(1),
                    destination: 0,
                },
            ]);
        }
        code.extend([
            Instruction::Lea {
                source: Ea::Displacement(6, -16),
                destination: 1,
            },
            mov(Ea::A(0), Ea::D(0)),
            mov(Ea::A(1), Ea::D(1)),
            alu(Alu::Sub, 1, 0),
            mov(Ea::A(1), slot(0)),
            mov(Ea::D(0), slot(4)),
            call(PlatformRoutineId::WriteSpan),
            Instruction::Unlink(6),
            Instruction::Rts,
        ]);
        self.block(end, code);
        Ok(())
    }
}
fn byte(source: Ea, destination: Ea) -> Instruction {
    Instruction::Move {
        width: Width::Byte,
        source,
        destination,
    }
}
fn local_branch(condition: Condition, target: MachineBlockId) -> Instruction {
    Instruction::Branch {
        condition,
        target: Address::new(Target::Block(target)),
    }
}
