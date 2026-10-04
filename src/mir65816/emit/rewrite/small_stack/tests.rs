use super::super::super::{layout, replay, selected::*, tracked::TrackedEmitter65816};
use super::*;

fn fixture(n: u16, subtract: bool, flag: Option<Branch>, split: bool) -> Code {
    super::probe(n, subtract, flag, split, false)
}

#[test]
fn bounded_steps_replay_and_invalidate_old_analysis_sites() {
    for subtract in [false, true] {
        for n in 0..=4 {
            let original = fixture(n, subtract, None, false);
            let old = original.selected.as_ref().unwrap().site(Node(0)).unwrap();
            let result = apply(original.clone(), false).unwrap();
            if (1..=3).contains(&n) {
                assert_eq!(
                    original.bytes.len() - result.bytes.len(),
                    usize::from(4 - n)
                );
                assert!(result.selected.as_ref().unwrap().validate(old).is_err());
                assert_eq!(
                    &result.bytes[1..1 + usize::from(n)],
                    vec![if subtract { 0x3a } else { 0x1a }; usize::from(n)]
                );
            } else {
                replay::equivalent(&original, &result).unwrap();
                assert!(result.selected.as_ref().unwrap().validate(old).is_ok());
            }
            let fresh = replay::emit(result.selected.as_ref().unwrap(), false).unwrap();
            replay::equivalent(&result, &layout::finalize(fresh, true).unwrap()).unwrap();
        }
    }
}

#[test]
fn live_carry_overflow_and_control_flow_entries_block_replacement() {
    for subtract in [false, true] {
        for flag in [Branch::CarryClear, Branch::OverflowClear] {
            let original = fixture(2, subtract, Some(flag), false);
            replay::equivalent(&original, &apply(original.clone(), false).unwrap()).unwrap();
        }
        // A label after TSC is a protected entry even on fallthrough.
        let original = fixture(2, subtract, None, true);
        replay::equivalent(&original, &apply(original.clone(), false).unwrap()).unwrap();
    }
}

#[test]
fn forged_steps_and_stack_equations_are_rejected() {
    let original = fixture(2, true, None, false);
    let s = original.selected.as_ref().unwrap();
    let mut plan = candidate(s, 2).unwrap();
    plan.replacement = vec![Instruction::Implied(Implied::IncA); 2];
    let mut code = original.clone();
    assert!(
        Driver::new(1)
            .apply(&mut code, &plan, false)
            .into_result()
            .is_err()
    );
    replay::equivalent(&original, &code).unwrap();
    let result = apply(original, false).unwrap();
    let mut records = result.selected.as_ref().unwrap().records().to_vec();
    records[2].after.stack_a = Some(17);
    assert!(super::super::super::analysis::cfg::SelectedCfg::build(&records).is_err());
}

#[test]
fn byte_steps_cannot_authorize_a_full_stack_equation() {
    let mut e = TrackedEmitter65816::default();
    e.a8();
    e.op(Implied::Tsc);
    e.op(Implied::IncA);
    assert_eq!(e.boundary().stack_a, None);
    e.a16();
    assert_eq!(e.boundary().stack_a, None);
}
