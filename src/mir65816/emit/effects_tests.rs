use super::*;
use crate::mir65816::{
    Mir65816CallActivation, Mir65816CallForm, Mir65816ModeState, Mir65816RegisterWidth,
};
use crate::target::{ByteOffset, ByteSize};

fn state(m: Width, index: Width) -> Environment {
    Environment {
        m,
        index,
        ..super::super::state::State65816::default().env
    }
}
fn fx(i: Instruction, m: Width, x: Width) -> InstructionEffects {
    i.effects(state(m, x))
}

#[test]
fn byte_operand_size_is_independent_of_data_and_index_widths() {
    for (m, mbytes) in [(Width::Byte, 1), (Width::Word, 2)] {
        for (x, xbytes) in [(Width::Byte, 1), (Width::Word, 2)] {
            for (op, access, memory) in [
                (
                    ByteOp::LdaStack,
                    Access::Read,
                    Memory::Stack {
                        displacement: 255,
                        bytes: mbytes,
                    },
                ),
                (
                    ByteOp::StaStack,
                    Access::Write,
                    Memory::Stack {
                        displacement: 255,
                        bytes: mbytes,
                    },
                ),
                (
                    ByteOp::LdaDp,
                    Access::Read,
                    Memory::DirectPage {
                        offset: 255,
                        bytes: mbytes,
                    },
                ),
                (
                    ByteOp::StaDp,
                    Access::Write,
                    Memory::DirectPage {
                        offset: 255,
                        bytes: mbytes,
                    },
                ),
                (
                    ByteOp::LdxDp,
                    Access::Read,
                    Memory::DirectPage {
                        offset: 255,
                        bytes: xbytes,
                    },
                ),
            ] {
                let e = fx(Instruction::Byte(op, 255), m, x);
                assert_eq!(e.memory, [MemoryEffect { access, memory }]);
            }
            let e = fx(Instruction::Byte(ByteOp::LdaDp, 32), m, x);
            assert_eq!(
                e.writes,
                Registers {
                    a: if mbytes == 1 { 0xff } else { 0xffff },
                    x: 0,
                    y: 0
                }
            );
            let e = fx(Instruction::Byte(ByteOp::LdxDp, 32), m, x);
            assert_eq!(e.writes.x, 0xffff); // Includes mandatory X8 zero extension.
            assert_eq!(e.environment_reads & (env::M | env::X), env::X);
        }
    }
}

#[test]
fn memory_arithmetic_and_rmw_have_exact_carry_and_ordered_accesses() {
    for width in [Width::Byte, Width::Word] {
        for op in [
            ByteOp::AdcDp,
            ByteOp::SbcDp,
            ByteOp::AdcStack,
            ByteOp::SbcStack,
        ] {
            let e = fx(Instruction::Byte(op, 8), width, Width::Word);
            assert_eq!((e.flag_reads, e.flag_writes), (C, NZCV));
            assert_eq!(e.reads.a, mask(width));
            assert_eq!(e.writes.a, mask(width));
            assert_eq!(e.memory.len(), 1);
            assert_eq!(e.memory[0].access, Access::Read);
            assert_ne!(e.environment_reads & env::DECIMAL, 0);
        }
        for op in [ByteOp::CmpDp, ByteOp::CmpStack] {
            let e = fx(Instruction::Byte(op, 8), width, Width::Word);
            assert_eq!((e.flag_reads, e.flag_writes), (0, NZ | C));
            assert_eq!(e.writes, Registers::default());
        }
        for op in [
            ByteOp::AndDp,
            ByteOp::AndStack,
            ByteOp::EorStack,
            ByteOp::OraDp,
            ByteOp::OraStack,
            ByteOp::EorDp,
        ] {
            let e = fx(Instruction::Byte(op, 8), width, Width::Word);
            assert_eq!((e.flag_reads, e.flag_writes), (0, NZ));
            assert_eq!(e.memory[0].access, Access::Read);
            if matches!(op, ByteOp::AndStack | ByteOp::OraStack | ByteOp::EorStack) {
                assert_eq!(
                    e.memory[0].memory,
                    Memory::Stack {
                        displacement: 8,
                        bytes: width.bytes().into(),
                    }
                );
                assert_eq!((e.reads.a, e.writes.a), (mask(width), mask(width)));
            }
        }
        for (op, carry) in [
            (ByteOp::AslDp, 0),
            (ByteOp::RolDp, C),
            (ByteOp::LsrDp, 0),
            (ByteOp::RorDp, C),
        ] {
            let e = fx(Instruction::Byte(op, 8), width, Width::Word);
            assert_eq!((e.flag_reads, e.flag_writes), (carry, NZ | C));
            assert_eq!(e.reads, Registers::default());
            assert_eq!(e.writes, Registers::default());
            assert_eq!(
                e.memory,
                [
                    MemoryEffect {
                        access: Access::Read,
                        memory: Memory::DirectPage {
                            offset: 8,
                            bytes: width.bytes().into()
                        }
                    },
                    MemoryEffect {
                        access: Access::Write,
                        memory: Memory::DirectPage {
                            offset: 8,
                            bytes: width.bytes().into()
                        }
                    }
                ]
            );
        }
    }
}

#[test]
fn immediate_forms_and_index_comparison_do_not_invent_memory_reads() {
    for (op, read, written, flags_read, flags_written) in [
        (ByteOp::LdaImm, 0, 0xff, 0, NZ),
        (ByteOp::AdcImm, 0xff, 0xff, C, NZCV),
        (ByteOp::SbcImm, 0xff, 0xff, C, NZCV),
        (ByteOp::CmpImm, 0xff, 0, 0, NZ | C),
        (ByteOp::EorImm, 0xff, 0xff, 0, NZ),
    ] {
        let e = fx(Instruction::Byte(op, 7), Width::Byte, Width::Word);
        assert_eq!(
            (e.reads.a, e.writes.a, e.flag_reads, e.flag_writes),
            (read, written, flags_read, flags_written)
        );
        assert!(e.memory.is_empty());
    }
    for (op, read, written, fr, fw) in [
        (WordOp::LdaImm, 0, 0xffff, 0, NZ),
        (WordOp::AdcImm, 0xffff, 0xffff, C, NZCV),
        (WordOp::SbcImm, 0xffff, 0xffff, C, NZCV),
        (WordOp::CmpImm, 0xffff, 0, 0, NZ | C),
        (WordOp::AndImm, 0xffff, 0xffff, 0, NZ),
        (WordOp::OraImm, 0xffff, 0xffff, 0, NZ),
        (WordOp::EorImm, 0xffff, 0xffff, 0, NZ),
    ] {
        let e = fx(Instruction::Word(op, 7), Width::Word, Width::Word);
        assert_eq!(
            (e.reads.a, e.writes.a, e.flag_reads, e.flag_writes),
            (read, written, fr, fw)
        );
        assert!(e.memory.is_empty());
    }
    for m in [Width::Byte, Width::Word] {
        let e = fx(Instruction::Word(WordOp::CpxImm, 0x8000), m, Width::Word);
        assert_eq!(
            e.reads,
            Registers {
                x: 0xffff,
                a: 0,
                y: 0
            }
        );
        assert_eq!((e.flag_reads, e.flag_writes), (0, NZ | C));
        assert_eq!(e.environment_reads & (env::M | env::X), env::X);
        let e = fx(Instruction::Word(WordOp::LdxImm, 0x8000), m, Width::Word);
        assert_eq!(
            e.writes,
            Registers {
                x: 0xffff,
                a: 0,
                y: 0
            }
        );
        assert_eq!(e.reads, Registers::default());
        assert_eq!((e.flag_reads, e.flag_writes), (0, NZ));
        assert_eq!(e.environment_reads & (env::M | env::X), env::X);
        assert!(e.memory.is_empty());
        let e = fx(Instruction::Word(WordOp::LdyImm, 2), m, Width::Word);
        assert_eq!(
            e.writes,
            Registers {
                y: 0xffff,
                a: 0,
                x: 0
            }
        );
    }
}

#[test]
fn transfers_hidden_a_and_index_updates_follow_their_own_widths() {
    for m in [Width::Byte, Width::Word] {
        for x in [Width::Byte, Width::Word] {
            for op in [Implied::Tax, Implied::Tay] {
                let e = fx(Instruction::Implied(op), m, x);
                assert_eq!(e.reads.a, mask(x));
                assert_eq!(
                    e.writes,
                    if op == Implied::Tax {
                        Registers {
                            x: 0xffff,
                            a: 0,
                            y: 0,
                        }
                    } else {
                        Registers {
                            y: 0xffff,
                            a: 0,
                            x: 0,
                        }
                    }
                );
                assert_eq!(e.flag_writes, NZ);
            }
            for op in [Implied::Txa, Implied::Tya] {
                let e = fx(Instruction::Implied(op), m, x);
                assert_eq!(e.writes.a, mask(m));
                assert_eq!(
                    if op == Implied::Txa {
                        e.reads.x
                    } else {
                        e.reads.y
                    },
                    mask(m)
                );
            }
            for op in [Implied::Inx, Implied::Dex] {
                let e = fx(Instruction::Implied(op), m, x);
                assert_eq!(
                    (e.reads.x, e.writes.x, e.flag_reads, e.flag_writes),
                    (mask(x), 0xffff, 0, NZ)
                );
                assert_eq!(e.environment_reads & (env::M | env::X), env::X);
            }
            let e = fx(Instruction::Implied(Implied::DecA), m, x);
            assert_eq!((e.reads.a, e.writes.a), (mask(m), mask(m)));
            let e = fx(Instruction::Implied(Implied::Xba), m, x);
            assert_eq!((e.reads.a, e.writes.a, e.flag_writes), (0xffff, 0xffff, NZ));
        }
    }
}

#[test]
fn status_bits_and_full_word_stack_transfers_are_protected() {
    for bit in [C, Z, V, N] {
        for op in [ByteOp::Rep, ByteOp::Sep] {
            let e = fx(Instruction::Byte(op, bit), Width::Word, Width::Word);
            assert_eq!(e.flag_writes, bit);
            assert_eq!(e.writes, Registers::default());
        }
    }
    for (bit, part) in [
        (4, env::I),
        (8, env::DECIMAL),
        (0x10, env::X),
        (0x20, env::M),
    ] {
        for op in [ByteOp::Rep, ByteOp::Sep] {
            let e = fx(Instruction::Byte(op, bit), Width::Word, Width::Word);
            assert_eq!(e.environment_writes, part);
            assert_eq!(e.flag_writes, 0);
            assert_eq!(
                e.writes.x,
                if bit == 0x10 && op == ByteOp::Sep {
                    0xff00
                } else {
                    0
                }
            );
            assert_eq!(e.writes.x, e.writes.y);
            assert!(e.barrier);
        }
    }
    for m in [Width::Byte, Width::Word] {
        let e = fx(Instruction::Implied(Implied::Tsc), m, Width::Word);
        assert_eq!((e.writes.a, e.flag_writes), (0xffff, NZ));
        let e = fx(Instruction::Implied(Implied::Tcs), m, Width::Word);
        assert_eq!(
            (e.reads.a, e.environment_writes, e.flag_writes),
            (0xffff, env::S, 0)
        );
        let e = fx(Instruction::Implied(Implied::Pha), m, Width::Word);
        assert_eq!(
            e.memory,
            [MemoryEffect {
                access: Access::Write,
                memory: Memory::Stack {
                    displacement: 1 - i32::from(m.bytes()),
                    bytes: m.bytes().into()
                }
            }]
        );
    }
    for (inst, displacement, bytes) in [
        (Instruction::Implied(Implied::Phk), 0, 1),
        (Instruction::PushReturn(Label(7)), -1, 2),
    ] {
        let e = fx(inst, Width::Word, Width::Word);
        assert_eq!(
            e.memory,
            [MemoryEffect {
                access: Access::Write,
                memory: Memory::Stack {
                    displacement,
                    bytes
                }
            }]
        );
    }
    for op in [Implied::Clc, Implied::Sec] {
        assert_eq!(
            fx(Instruction::Implied(op), Width::Word, Width::Word).flag_writes,
            C
        );
    }
    assert_eq!(
        fx(Instruction::Implied(Implied::Nop), Width::Word, Width::Word).writes,
        Registers::default()
    );
}

#[test]
fn indirect_accesses_read_pointer_bytes_and_keep_alias_uncertainty() {
    for m in [Width::Byte, Width::Word] {
        for op in [
            ByteOp::LdaIndirect,
            ByteOp::StaIndirect,
            ByteOp::LdaIndirectY,
            ByteOp::StaIndirectY,
        ] {
            let e = fx(Instruction::Byte(op, 0xff), m, Width::Word);
            let indexed_y = matches!(op, ByteOp::LdaIndirectY | ByteOp::StaIndirectY);
            assert_eq!(
                e.memory[0],
                MemoryEffect {
                    access: Access::Read,
                    memory: Memory::DirectPage {
                        offset: 255,
                        bytes: 3
                    }
                }
            );
            assert_eq!(
                e.memory[1],
                MemoryEffect {
                    access: if matches!(op, ByteOp::LdaIndirect | ByteOp::LdaIndirectY) {
                        Access::Read
                    } else {
                        Access::MayWrite
                    },
                    memory: Memory::IndirectLong {
                        pointer: 255,
                        indexed_y,
                        bytes: m.bytes()
                    }
                }
            );
            assert_eq!(e.reads.y, if indexed_y { 0xffff } else { 0 });
            assert!(e.barrier);
        }
        for op in [LongOp::Lda, LongOp::Sta] {
            let e = fx(Instruction::Long(op, 0x12ffff), m, Width::Word);
            assert_eq!(
                e.memory[0].memory,
                Memory::Long {
                    address: 0x12ffff,
                    bytes: m.bytes()
                }
            );
            assert!(e.barrier);
        }
        for op in [ReferenceOp::LdaLong, ReferenceOp::StaLong] {
            let e = fx(
                Instruction::Reference(op, Target::StackOverflow, 9, None),
                m,
                Width::Word,
            );
            assert_eq!(
                e.memory[0].memory,
                Memory::Symbol {
                    target: Target::StackOverflow,
                    addend: 9,
                    bytes: m.bytes()
                }
            );
            assert!(e.barrier);
        }
    }
    let e = fx(
        Instruction::Reference(ReferenceOp::LdaByte, Target::StackOverflow, 3, Some(2)),
        Width::Byte,
        Width::Word,
    );
    assert!(e.memory.is_empty());
}

fn plan(transfer: FarTransfer, result: ResultLocation) -> Mir65816CallPlan {
    let mode = Mir65816ModeState {
        native_mode: true,
        accumulator: Mir65816RegisterWidth::Bits16,
        index: Mir65816RegisterWidth::Bits16,
    };
    Mir65816CallPlan {
        convention: crate::nir::NirCallConvention::TargetPublic,
        native: Some(abi::NativeCallContract {
            boundary: abi::BOUNDARY,
            argument_bytes: ByteSize::new(4),
            transfer,
            caller_cleanup_bytes: ByteSize::new(5),
        }),
        arguments: vec![
            Mir65816AbiHome::StackArgument {
                offset: ByteOffset::new(0),
                size: ByteSize::new(2),
                alignment: ByteSize::new(2),
            },
            Mir65816AbiHome::StackArgument {
                offset: ByteOffset::new(2),
                size: ByteSize::new(2),
                alignment: ByteSize::new(2),
            },
        ],
        result: Some(Mir65816AbiHome::NativeResult(result)),
        outgoing_bytes: ByteSize::new(5),
        code_pointer_width: ByteSize::new(3),
        call_form: if transfer == FarTransfer::Jsl {
            Mir65816CallForm::FarJsl
        } else {
            Mir65816CallForm::FarStackRtl
        },
        mode_before: mode,
        mode_after: mode,
        activation: Mir65816CallActivation::Fresh,
        net_stack_delta: 0,
    }
}
#[test]
fn call_summaries_read_arguments_before_clobbers_and_preserve_result_contracts() {
    for result in [
        ResultLocation::A8ZeroExtended,
        ResultLocation::A16,
        ResultLocation::A16X8ZeroExtended,
        ResultLocation::A16X16,
    ] {
        let expected = Registers {
            a: 0xffff,
            x: match result {
                ResultLocation::A8ZeroExtended | ResultLocation::A16 => 0,
                ResultLocation::A16X8ZeroExtended | ResultLocation::A16X16 => 0xffff,
            },
            y: 0,
        };
        for transfer in [FarTransfer::Jsl, FarTransfer::StackRtl] {
            let p = plan(transfer, result);
            let c = CallContract::from_plan(&p, transfer).unwrap();
            let indirect = transfer == FarTransfer::StackRtl;
            let e = fx(
                if indirect {
                    Instruction::IndirectTransfer(Some(c))
                } else {
                    Instruction::NativeCall(Target::Routine(super::super::RoutineId(1)), c)
                },
                Width::Word,
                Width::Word,
            );
            let first = if indirect { 7 } else { 1 };
            assert_eq!(
                e.memory[1].memory,
                Memory::Stack {
                    displacement: first,
                    bytes: 2
                }
            );
            assert_eq!(
                e.memory[2].memory,
                Memory::Stack {
                    displacement: first + 2,
                    bytes: 2
                }
            );
            assert_eq!(
                e.memory[3],
                MemoryEffect {
                    access: Access::Read,
                    memory: Memory::Unknown
                }
            );
            assert_eq!(
                e.memory[4],
                MemoryEffect {
                    access: Access::MayWrite,
                    memory: Memory::Unknown
                }
            );
            assert_eq!(
                e.memory[5],
                MemoryEffect {
                    access: Access::MayWrite,
                    memory: Memory::DirectPage {
                        offset: 0,
                        bytes: 64
                    }
                }
            );
            assert_eq!(
                e.memory[6].memory,
                Memory::Stack {
                    displacement: if indirect { 4 } else { -2 },
                    bytes: 3
                }
            );
            assert_eq!(e.writes, expected);
            assert_eq!(e.writes.a | e.clobbers.a, 0xffff);
            assert_eq!(e.writes.x | e.clobbers.x, 0xffff);
            assert_eq!(e.flag_clobbers, NZCV);
            assert!(e.barrier);
        }
        let e = fx(
            Instruction::NativeReturn(Some(result)),
            Width::Word,
            Width::Word,
        );
        assert_eq!(e.reads, expected);
        assert_eq!(e.control, Control::Return);
    }
    let mut p = plan(FarTransfer::Jsl, ResultLocation::A16);
    assert!(CallContract::from_plan(&p, FarTransfer::StackRtl).is_err());
    p.outgoing_bytes = ByteSize::new(2);
    assert!(CallContract::from_plan(&p, FarTransfer::Jsl).is_err());
    assert!(CallContract::result(Some(Mir65816AbiHome::Accumulator)).is_err());
}
#[test]
fn branches_and_unannotated_transfers_are_never_empty_effects() {
    for (op, flag) in [
        (Branch::Plus, N),
        (Branch::Minus, N),
        (Branch::OverflowClear, V),
        (Branch::CarryClear, C),
        (Branch::CarrySet, C),
        (Branch::NotEqual, Z),
        (Branch::Equal, Z),
    ] {
        let e = fx(Instruction::Branch(op, Label(8)), Width::Word, Width::Word);
        assert_eq!((e.flag_reads, e.flag_writes), (flag, 0));
        assert_eq!(
            e.control,
            Control::Branch {
                predicate: op.opcode(),
                target: Label(8)
            }
        );
    }
    let e = fx(
        Instruction::Reference(ReferenceOp::Jml, Target::Label(Label(9)), 0, None),
        Width::Word,
        Width::Word,
    );
    assert_eq!(e.control, Control::Jump(Target::Label(Label(9))));
    for inst in [
        Instruction::Reference(
            ReferenceOp::Jsl,
            Target::Routine(super::super::RoutineId(1)),
            0,
            None,
        ),
        Instruction::IndirectTransfer(None),
    ] {
        let e = fx(inst, Width::Word, Width::Word);
        assert_eq!(e.reads, Registers::ALL);
        assert_eq!(e.clobbers, Registers::ALL);
        assert_eq!(e.flag_reads, NZCV);
        assert!(e.barrier);
    }
    assert_eq!(
        fx(Instruction::Implied(Implied::Rtl), Width::Word, Width::Word).reads,
        Registers::ALL
    );
    assert_eq!(
        fx(Instruction::NativeReturn(None), Width::Word, Width::Word).reads,
        Registers::default()
    );
}

#[test]
fn symbolic_indexed_load_reads_x_and_keeps_dynamic_memory_identity() {
    for m in [Width::Byte, Width::Word] {
        for x in [Width::Byte, Width::Word] {
            let target = Target::StackOverflow;
            let e = fx(
                Instruction::Reference(ReferenceOp::LdaLongX, target, 0, None),
                m,
                x,
            );
            assert_eq!(e.reads.x, x.mask());
            assert_eq!(e.writes.a, m.mask());
            assert_eq!(
                e.memory[0].memory,
                Memory::SymbolIndexedX {
                    target,
                    addend: 0,
                    bytes: m.bytes()
                }
            );
            assert_eq!(e.memory[0].access, Access::Read);
            assert!(e.barrier);
        }
    }
}
