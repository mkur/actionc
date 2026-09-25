mod support;
use actionc::mir65816::{
    Mir65816Op, Mir65816Value,
    emit::{self, proof},
    image, o65 as format,
};
use actionc_vm::native65816::Access;
use support::*;
const SOURCE: &str = r#"
ADDRESS input=$7100
VOLATILE ADDRESS io=$d000
ADDRESS ARRAY out=$7200
PROC Change() input=ADDRESS($abcdef) RETURN
ADDRESS FUNC Cast(ADDRESS p) RETURN(ADDRESS(BYTE POINTER(p)))
ADDRESS FUNC Across(ADDRESS p)
 ADDRESS saved
 saved=ADDRESS(BYTE POINTER(p)) Change()
RETURN(ADDRESS(BYTE POINTER(saved)))
PROC Main()
 ADDRESS POINTER cell
 cell=ADDRESS POINTER($12ffff)
 out(0)=Cast(input) out(1)=Across(input) out(2)=Cast(io) out(3)=Cast(cell^)
RETURN
"#;
fn check(h: &Harness, value: u32) {
    for i in 0..4 {
        assert_eq!(h.bus.value(0x7200 + 3 * i, 3), value);
    }
    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x720c]), (0xa5, 0xa5));
    let expected: Vec<_> = (0xd000..0xd003)
        .chain(0x12ffff..0x130002)
        .map(|a| (a, Access::Read))
        .collect();
    assert_eq!(
        h.bus
            .trace
            .iter()
            .map(|&(_, a, k)| (a, k))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!((h.bus.ram[0x12fffe], h.bus.ram[0x130002]), (0xa5, 0xa5));
}
fn initialize(h: &mut Harness, input: u32, value: u32) {
    h.bus.ram[input as usize..input as usize + 3].copy_from_slice(&value.to_le_bytes()[..3]);
    h.bus.ram[0x71ff..0x720d].fill(0xa5);
    h.bus.map(0xd000, &value.to_le_bytes()[..3], true);
    h.bus.map(0x12fffe, &[0xa5, 0, 0, 0, 0xa5], true);
    h.bus.ram[0x12ffff..0x130002].copy_from_slice(&value.to_le_bytes()[..3]);
    h.bus.watched.extend(0xd000..0xd003);
    h.bus.watched.extend(0x12fffe..0x130003);
}
#[test]
fn pointer_coalescing_keeps_all_lanes_external_reads_and_captures_across_calls() {
    for optimize in [false, true] {
        let image = compile(SOURCE, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&SOURCE.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        for value in [0, 0xffffff, 0x12ffff, 0x800000, 0xabcdef]
            .into_iter()
            .chain((0..24).map(|b| 1 << b))
        {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                initialize(&mut h, 0x7100, value);
                h.run();
                h.guards(mask);
                check(&h, value);
                assert_eq!(h.bus.value(0x7100, 3), 0xabcdef);
            }
        }
    }
}
#[test]
fn coalesced_pointer_casts_have_empty_spans_and_exact_replay() {
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let c = p.compile(&layout()).unwrap();
        let mut removed = 0;
        for r in &c.machine.prepared.routines {
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if let Mir65816Op::Cast {
                        dest,
                        from,
                        to,
                        value: Mir65816Value::Temp(source, _),
                        ..
                    } = op
                    {
                        if from.get() == 3
                            && to.get() == 3
                            && m.frame.temps[dest] == m.frame.temps[source]
                        {
                            assert!(m.code.mir_spans[&(b.id, i)].is_empty());
                            removed += 1;
                        }
                    }
                }
            }
        }
        assert!(removed >= 2, "only {removed} identity casts removed");
        let (a, old) = proof::materialize_reference(&p.mir, true).unwrap();
        let (b, new) = proof::materialize_replayed(&p.mir, true).unwrap();
        assert_eq!(
            image::link(&p.mir, &a, &layout())
                .unwrap()
                .to_json()
                .unwrap(),
            c.image.to_json().unwrap()
        );
        for ((a, b), (old, new)) in a.routines.iter().zip(&b.routines).zip(old.iter().zip(&new)) {
            proof::compare_replay_output(&a.code, &b.code).unwrap();
            assert_eq!(old.snapshots, new.snapshots);
        }
        let _ = emit::materialize(&p.mir).unwrap();
    }
}
#[test]
fn pointer_coalescing_executes_after_independent_o65_placements() {
    let source = SOURCE.replace("input=$7100", "input");
    for optimize in [false, true] {
        let bytes = o65::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            for value in [0, 0x12ffff, 0xabcdef, 0xffffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    initialize(&mut h, o65::object(&image, "input"), value);
                    h.run();
                    h.guards(mask);
                    check(&h, value);
                }
            }
        }
    }
}
