//! One Shell invocation, with a saved entry frame and one terminal cleanup path.
use super::*;
use crate::mir68k::{Mir68kProgram, machine::*, materialize};

const FRAME: i16 = -64;
const SAVED_FRAME: i64 = 0;
const DOS: i64 = 4;
const OUTPUT: i64 = 8;
const STATUS: i64 = 12;

#[path = "amiga_console.rs"]
mod console;

pub fn materialize(
    program: &Mir68kProgram,
    options: materialize::Options,
) -> Result<MachineProgram, String> {
    materialize_with_bindings(program, options, &BTreeMap::new())
}

pub fn materialize_with_bindings(
    program: &Mir68kProgram,
    options: materialize::Options,
    bindings: &BTreeMap<crate::nir::RoutineId, ConsoleService>,
) -> Result<MachineProgram, String> {
    for id in bindings.keys() {
        if !program
            .routines
            .iter()
            .any(|r| r.id == *id && r.entry.external)
        {
            return Err("Amiga binding must identify an external routine".into());
        }
    }
    let mut machine = materialize::materialize_program(
        program,
        materialize::Options {
            relax_branches: false,
            ..options
        },
        Some(bindings),
    )?;
    compose(program, &mut machine, bindings)?;
    if options.relax_branches {
        crate::mir68k::branch_relaxation::relax(&mut machine)?;
    }
    Ok(machine)
}

fn compose(
    program: &Mir68kProgram,
    machine: &mut MachineProgram,
    bindings: &BTreeMap<crate::nir::RoutineId, ConsoleService>,
) -> Result<(), String> {
    let mut builder = Builder {
        next: machine
            .blocks
            .iter()
            .map(|b| b.id.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("too many platform blocks")?,
        blocks: vec![],
        platform: Platform::default(),
    };
    builder.platform.entry = Some(PlatformRoutineId::Startup);
    builder.platform.data.insert(
        PlatformDataId::State,
        PlatformData {
            bytes: vec![],
            zero_fill: 16,
        },
    );
    builder.platform.data.insert(
        PlatformDataId::LibraryName,
        PlatformData {
            bytes: b"dos.library\0".to_vec(),
            zero_fill: 0,
        },
    );
    builder.startup(program.entry.ok_or("missing Amiga entry")?)?;
    builder.cleanup()?;
    builder.span()?;
    for service in bindings
        .values()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        builder.console(service)?;
    }
    for reason in RuntimeFault::ALL {
        let message = format!("Action! {}\n", reason.name()).into_bytes();
        let length = message.len() as u32;
        builder.platform.data.insert(
            PlatformDataId::FaultMessage(reason),
            PlatformData {
                bytes: message,
                zero_fill: 0,
            },
        );
        builder.routine(
            PlatformRoutineId::Fault(reason),
            vec![
                mov(Ea::Immediate(20), state(STATUS)),
                mov(state(SAVED_FRAME), Ea::A(6)),
                Instruction::Lea {
                    source: Ea::Displacement(6, FRAME),
                    destination: 7,
                },
                mov(state(OUTPUT), Ea::D(0)),
                cmp(0, 0),
                branch(Condition::Equal, PlatformRoutineId::Cleanup),
                mov(
                    Ea::ImmediateAddress(Address::new(Target::PlatformData(
                        PlatformDataId::FaultMessage(reason),
                    ))),
                    slot(0),
                ),
                mov(Ea::Immediate(length), slot(4)),
                call(PlatformRoutineId::WriteSpan),
                jump(PlatformRoutineId::Cleanup),
            ],
        )?;
    }
    for call in [
        LibraryCall::OpenLibrary,
        LibraryCall::CloseLibrary,
        LibraryCall::Output,
        LibraryCall::Write,
    ] {
        builder.routine(
            PlatformRoutineId::Library(call),
            library_adapter(call, &vec![Argument::LONG; call.argument_count()])?,
        )?;
    }
    // All target instructions participate in one final code layout/relaxation.
    builder.blocks.append(&mut machine.blocks);
    machine.blocks = builder.blocks;
    machine.platform = builder.platform;
    Ok(())
}

struct Builder {
    next: u32,
    blocks: Vec<MachineBlock>,
    platform: Platform,
}
impl Builder {
    fn label(&mut self) -> Result<MachineBlockId, String> {
        let id = MachineBlockId(self.next);
        self.next = self.next.checked_add(1).ok_or("too many platform blocks")?;
        Ok(id)
    }
    fn block(&mut self, id: MachineBlockId, instructions: Vec<Instruction>) {
        self.blocks.push(MachineBlock { id, instructions });
    }
    fn routine(
        &mut self,
        id: PlatformRoutineId,
        instructions: Vec<Instruction>,
    ) -> Result<(), String> {
        let block = self.label()?;
        if self.platform.routines.insert(id, block).is_some() {
            return Err("duplicate platform routine".into());
        }
        self.block(block, instructions);
        Ok(())
    }
    fn startup(&mut self, entry: crate::nir::RoutineId) -> Result<(), String> {
        let mut code = vec![Instruction::Link {
            register: 6,
            displacement: FRAME,
        }];
        for (index, register) in saved().enumerate() {
            code.push(mov(register, Ea::Displacement(6, -4 * (index as i16 + 1))));
        }
        code.push(mov(Ea::A(6), state(SAVED_FRAME)));
        for offset in [DOS, OUTPUT, STATUS] {
            code.push(mov(Ea::Immediate(0), state(offset)));
        }
        code.extend([
            mov(Ea::Absolute(Address::absolute(4)), slot(0)),
            mov(
                Ea::ImmediateAddress(Address::new(Target::PlatformData(
                    PlatformDataId::LibraryName,
                ))),
                slot(4),
            ),
            mov(Ea::Immediate(40), slot(8)), // AmigaOS 3.1 library version floor.
            call(PlatformRoutineId::Library(LibraryCall::OpenLibrary)),
            mov(Ea::A(0), state(DOS)),
            mov(Ea::A(0), Ea::D(0)),
            cmp(0, 0),
            branch(Condition::Equal, PlatformRoutineId::Failure),
            mov(state(DOS), slot(0)),
            call(PlatformRoutineId::Library(LibraryCall::Output)),
            mov(Ea::D(0), state(OUTPUT)),
            cmp(0, 0),
            branch(Condition::Equal, PlatformRoutineId::Failure),
            Instruction::Jsr(Ea::Absolute(Address::new(Target::Routine(entry)))),
            jump(PlatformRoutineId::Cleanup),
        ]);
        self.routine(PlatformRoutineId::Startup, code)
    }
    fn cleanup(&mut self) -> Result<(), String> {
        self.routine(
            PlatformRoutineId::Failure,
            vec![
                mov(Ea::Immediate(20), state(STATUS)),
                jump(PlatformRoutineId::Cleanup),
            ],
        )?;
        self.routine(
            PlatformRoutineId::Cleanup,
            vec![
                mov(state(SAVED_FRAME), Ea::A(6)),
                Instruction::Lea {
                    source: Ea::Displacement(6, FRAME),
                    destination: 7,
                },
                mov(state(DOS), Ea::D(0)),
                cmp(0, 0),
                branch(Condition::Equal, PlatformRoutineId::Finish),
                mov(Ea::Absolute(Address::absolute(4)), slot(0)),
                mov(Ea::D(0), slot(4)),
                call(PlatformRoutineId::Library(LibraryCall::CloseLibrary)),
                mov(Ea::Immediate(0), state(DOS)),
                jump(PlatformRoutineId::Finish),
            ],
        )?;
        let mut code = vec![mov(state(STATUS), Ea::D(0))];
        for (index, register) in saved().enumerate() {
            code.push(mov(Ea::Displacement(6, -4 * (index as i16 + 1)), register));
        }
        code.extend([Instruction::Unlink(6), Instruction::Rts]);
        self.routine(PlatformRoutineId::Finish, code)
    }
    fn span(&mut self) -> Result<(), String> {
        let again = self.label()?;
        let done = self.label()?;
        self.routine(
            PlatformRoutineId::WriteSpan,
            vec![
                Instruction::Link {
                    register: 6,
                    displacement: -24,
                },
                mov(Ea::D(2), Ea::Displacement(6, -4)),
                mov(Ea::D(3), Ea::Displacement(6, -8)),
                mov(Ea::Displacement(6, 8), Ea::D(2)),
                mov(Ea::Displacement(6, 12), Ea::D(3)),
                cmp(0, 3),
                branch(Condition::Minus, PlatformRoutineId::Failure),
                Instruction::Jump(Ea::Absolute(Address::new(Target::Block(again)))),
            ],
        )?;
        self.block(
            again,
            vec![
                cmp(0, 3),
                Instruction::Branch {
                    condition: Condition::Equal,
                    target: Address::new(Target::Block(done)),
                },
                mov(state(DOS), slot(0)),
                mov(state(OUTPUT), slot(4)),
                mov(Ea::D(2), slot(8)),
                mov(Ea::D(3), slot(12)),
                call(PlatformRoutineId::Library(LibraryCall::Write)),
                cmp(0, 0),
                branch(Condition::LessOrEqual, PlatformRoutineId::Failure),
                alu(Alu::Compare, 3, 0),
                branch(Condition::High, PlatformRoutineId::Failure),
                alu(Alu::Add, 0, 2),
                alu(Alu::Sub, 0, 3),
                Instruction::Jump(Ea::Absolute(Address::new(Target::Block(again)))),
            ],
        );
        self.block(
            done,
            vec![
                mov(Ea::Displacement(6, -4), Ea::D(2)),
                mov(Ea::Displacement(6, -8), Ea::D(3)),
                Instruction::Unlink(6),
                Instruction::Rts,
            ],
        );
        Ok(())
    }
}
fn saved() -> impl Iterator<Item = Ea> {
    (2..8).map(Ea::D).chain((2..6).map(Ea::A))
}
fn mov(source: Ea, destination: Ea) -> Instruction {
    Instruction::Move {
        width: Width::Long,
        source,
        destination,
    }
}
fn alu(operation: Alu, source: u8, destination: u8) -> Instruction {
    Instruction::Alu {
        operation,
        width: Width::Long,
        source,
        destination,
    }
}
fn cmp(value: u32, destination: u8) -> Instruction {
    Instruction::CompareImmediate {
        width: Width::Long,
        value,
        destination,
    }
}
fn state(offset: i64) -> Ea {
    Ea::Absolute(Address {
        target: Target::PlatformData(PlatformDataId::State),
        addend: offset,
    })
}
fn slot(offset: i16) -> Ea {
    Ea::Displacement(7, offset)
}
fn address(id: PlatformRoutineId) -> Address {
    Address::new(Target::PlatformRoutine(id))
}
fn call(id: PlatformRoutineId) -> Instruction {
    Instruction::Jsr(Ea::Absolute(address(id)))
}
fn jump(id: PlatformRoutineId) -> Instruction {
    Instruction::Jump(Ea::Absolute(address(id)))
}
fn branch(condition: Condition, id: PlatformRoutineId) -> Instruction {
    Instruction::Branch {
        condition,
        target: address(id),
    }
}
