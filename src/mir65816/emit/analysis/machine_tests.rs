use super::super::super::{
    AllocatedFrame,
    effects::{C, N, V, Z},
    selected::*,
    state::{State65816, Width},
    tracked::TrackedEmitter65816,
};
use super::super::home_tests::Graph;
use super::*;
use crate::mir65816::abi::ResultLocation;
use crate::nir::RoutineId;

fn linear(forms: &[Instruction], m: Width, index: Width, end: MachineLive) -> MachineLiveness {
    let graph = Graph::new(
        forms.len() + 1,
        &(0..forms.len()).map(|n| (n, n + 1)).collect::<Vec<_>>(),
    );
    let mut env = State65816::default().env;
    env.m = m;
    env.index = index;
    let effects = forms
        .iter()
        .enumerate()
        .map(|(n, f)| (Node(n), f.effects(env)))
        .collect();
    MachineLiveness::solve(&graph, &effects, &[(Node(forms.len()), end)].into())
}
#[test]
fn flags_are_independent_through_diamond_and_overwrites() {
    let graph = Graph::new(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    for flag in [N, Z, C, V] {
        let effects = [
            (
                Node(1),
                InstructionEffects {
                    flag_reads: flag,
                    ..Default::default()
                },
            ),
            (
                Node(2),
                InstructionEffects {
                    flag_writes: NZCV,
                    ..Default::default()
                },
            ),
        ]
        .into();
        let live = MachineLiveness::solve(&graph, &effects, &BTreeMap::new());
        assert_eq!(live.before(Node(0)).unwrap().flags, flag);
        assert_eq!(live.before(Node(2)).unwrap().flags, 0);
    }
}
#[test]
fn lanes_and_flags_flow_around_backedges_but_not_unreachable_nodes() {
    let graph = Graph::new(5, &[(0, 1), (1, 2), (2, 1), (2, 3)]);
    let effects = [
        (
            Node(1),
            InstructionEffects {
                reads: Registers {
                    x: 0xff00,
                    ..Default::default()
                },
                flag_reads: V,
                ..Default::default()
            },
        ),
        (
            Node(2),
            InstructionEffects {
                flag_writes: V,
                ..Default::default()
            },
        ),
        (
            Node(4),
            InstructionEffects {
                reads: Registers::ALL,
                flag_reads: NZCV,
                ..Default::default()
            },
        ),
    ]
    .into();
    let live = MachineLiveness::solve(&graph, &effects, &BTreeMap::new());
    assert_eq!(
        live.before(Node(0)).unwrap().registers,
        Registers {
            x: 0xff00,
            ..Default::default()
        }
    );
    assert_eq!(live.after(Node(2)).unwrap().flags, V);
    assert!(live.before(Node(4)).is_err());
}
#[test]
fn byte_load_preserves_hidden_a_high_and_xba_reads_both_lanes() {
    let forms = [
        Instruction::Byte(ByteOp::LdaImm, 7),
        Instruction::Implied(Implied::Xba),
    ];
    let live = linear(
        &forms,
        Width::Byte,
        Width::Word,
        MachineLive {
            registers: Registers {
                a: 0xffff,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(live.before(Node(0)).unwrap().registers.a, 0xff00);
    assert_eq!(live.before(Node(1)).unwrap().registers.a, 0xffff);
    assert!(
        !live
            .before(Node(0))
            .unwrap()
            .register_live(RegisterLane::ALow)
    );
    assert!(
        live.before(Node(0))
            .unwrap()
            .register_live(RegisterLane::AHigh)
    );
}
#[test]
fn index_narrowing_kills_only_high_bytes_and_byte_index_writes_define_zero_high() {
    let end = MachineLive {
        registers: Registers {
            x: 0xffff,
            y: 0xffff,
            ..Default::default()
        },
        ..Default::default()
    };
    let live = linear(
        &[Instruction::Byte(ByteOp::Sep, 0x10)],
        Width::Word,
        Width::Word,
        end,
    );
    assert_eq!(
        live.before(Node(0)).unwrap().registers,
        Registers {
            x: 0xff,
            y: 0xff,
            a: 0
        }
    );
    let live = linear(
        &[Instruction::Byte(ByteOp::LdxDp, 0)],
        Width::Word,
        Width::Byte,
        end,
    );
    assert_eq!(live.before(Node(0)).unwrap().registers.x, 0);
    assert_eq!(live.before(Node(0)).unwrap().registers.y, 0xffff);
}
#[test]
fn adc_and_sbc_need_carry_input_and_do_not_need_old_result_flags() {
    for op in [WordOp::AdcImm, WordOp::SbcImm] {
        let live = linear(
            &[Instruction::Implied(Implied::Clc), Instruction::Word(op, 1)],
            Width::Word,
            Width::Word,
            MachineLive {
                flags: NZCV,
                ..Default::default()
            },
        );
        assert_eq!(live.before(Node(1)).unwrap().flags, C);
        assert_eq!(live.before(Node(0)).unwrap().flags, 0);
        assert_eq!(live.before(Node(1)).unwrap().registers.a, 0xffff);
    }
}
#[test]
fn cmp_defines_branch_flags_but_preserves_v() {
    for (branch, flag) in [
        (Branch::Plus, N),
        (Branch::Equal, Z),
        (Branch::CarryClear, C),
    ] {
        let live = linear(
            &[
                Instruction::Word(WordOp::CmpImm, 3),
                Instruction::Branch(branch, super::super::super::Label(0)),
            ],
            Width::Word,
            Width::Word,
            MachineLive {
                flags: V,
                ..Default::default()
            },
        );
        assert_eq!(live.after(Node(0)).unwrap().flags, flag | V);
        assert_eq!(live.before(Node(0)).unwrap().flags, V);
    }
}
#[test]
fn inx_to_txa_uses_index_lanes_and_overwrites_a_and_nz() {
    for (index, mask) in [(Width::Byte, 0xff), (Width::Word, 0xffff)] {
        let live = linear(
            &[
                Instruction::Implied(Implied::Inx),
                Instruction::Implied(Implied::Txa),
            ],
            Width::Word,
            index,
            MachineLive {
                registers: Registers {
                    a: 0xffff,
                    ..Default::default()
                },
                flags: N | Z,
                ..Default::default()
            },
        );
        assert_eq!(
            live.before(Node(0)).unwrap().registers,
            Registers {
                x: mask,
                ..Default::default()
            }
        );
        assert_eq!(live.after(Node(0)).unwrap().flags, 0);
    }
}
#[test]
fn tsc_tcs_are_full_width_and_environment_remains_protected() {
    let live = linear(
        &[
            Instruction::Implied(Implied::Tsc),
            Instruction::Implied(Implied::Tcs),
        ],
        Width::Byte,
        Width::Word,
        MachineLive {
            environment: env::ALL,
            ..Default::default()
        },
    );
    assert_eq!(live.before(Node(1)).unwrap().registers.a, 0xffff);
    assert_eq!(live.before(Node(0)).unwrap().registers.a, 0);
    assert_eq!(live.before(Node(0)).unwrap().environment, env::ALL);
}
#[test]
fn call_inputs_are_added_after_clobbers_kill_the_old_values() {
    let graph = Graph::new(1, &[]);
    let effects = [(
        Node(0),
        InstructionEffects {
            reads: Registers {
                x: 0xff,
                ..Default::default()
            },
            clobbers: Registers::ALL,
            flag_reads: C,
            flag_clobbers: NZCV,
            ..Default::default()
        },
    )]
    .into();
    let boundary = [(
        Node(0),
        MachineLive {
            registers: Registers::ALL,
            flags: NZCV,
            ..Default::default()
        },
    )]
    .into();
    let live = MachineLiveness::solve(&graph, &effects, &boundary);
    assert_eq!(
        live.before(Node(0)).unwrap().registers,
        Registers {
            x: 0xff,
            ..Default::default()
        }
    );
    assert_eq!(live.before(Node(0)).unwrap().flags, C);
}
#[test]
fn native_return_boundaries_keep_all_defined_result_bits_including_zero_extension() {
    for result in [
        None,
        Some(ResultLocation::A8ZeroExtended),
        Some(ResultLocation::A16),
        Some(ResultLocation::A16X8ZeroExtended),
        Some(ResultLocation::A16X16),
    ] {
        let mut e = TrackedEmitter65816::default();
        e.native_return(result.map(crate::mir65816::Mir65816AbiHome::NativeResult))
            .unwrap();
        let frame = AllocatedFrame {
            extent: 0,
            spill_bytes: 0,
            peak_below_entry: 0,
            temps: BTreeMap::new(),
            edge_copies: vec![],
        };
        let code = e.finish_selected(RoutineId(0), &frame, None).unwrap();
        let selected = code.selected.as_ref().unwrap();
        let live = MachineLiveness::analyze(selected);
        let expected = Registers {
            a: if result.is_some() { 0xffff } else { 0 },
            x: if matches!(
                result,
                Some(ResultLocation::A16X8ZeroExtended | ResultLocation::A16X16)
            ) {
                0xffff
            } else {
                0
            },
            y: 0,
        };
        for (n, r) in selected.records().iter().enumerate() {
            if matches!(r.action, Action::ReturnExit) {
                assert_eq!(
                    live.before(Node(n)).unwrap(),
                    MachineLive {
                        registers: expected,
                        flags: 0,
                        environment: env::ALL
                    }
                );
            }
        }
    }
}
