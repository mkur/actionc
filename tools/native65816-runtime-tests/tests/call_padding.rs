mod support;
use actionc::{
    mir65816::{Mir65816Op, abi, image::AssemblyImport},
    nir::runtime_symbol_id,
};
use actionc_vm::native65816::Inputs;
use serde_json::json;
use std::path::Path;
use support::*;

const OBSERVE: u32 = 0x041000;
const RECORD: usize = 0x7200;

struct Case {
    name: &'static str,
    parameters: String,
    arguments: String,
    expected: Vec<u8>,
    padding: Vec<usize>,
}

fn cases() -> Vec<Case> {
    let mut cases: Vec<_> = [
        ("empty", "", "", vec![0], vec![0]),
        ("byte", "BYTE a", "$B7", vec![0xb7], vec![]),
        ("zero", "BYTE a", "0", vec![0], vec![]),
        ("word", "CARD a", "$3456", vec![0x56, 0x34, 0], vec![2]),
        (
            "pointer",
            "BYTE POINTER p",
            "BYTE POINTER($AB789A)",
            vec![0x9a, 0x78, 0xab],
            vec![],
        ),
        (
            "captured_pointer",
            "ADDRESS p",
            "input",
            vec![0x9a, 0x78, 0xab],
            vec![],
        ),
        (
            "banked_pointer",
            "ADDRESS p",
            "input^",
            vec![0x9a, 0x78, 0xab],
            vec![],
        ),
        (
            "mixed",
            "BYTE a CARD b BYTE POINTER p LONGINT c",
            "$12,$3456,BYTE POINTER($AB789A),LONGINT($BCDEF012)",
            vec![
                0x12, 0, 0x56, 0x34, 0x9a, 0x78, 0xab, 0, 0x12, 0xf0, 0xde, 0xbc, 0,
            ],
            vec![1, 7, 12],
        ),
        (
            "holes_only",
            "BYTE a CARD b BYTE c",
            "$12,$3456,$B7",
            vec![0x12, 0, 0x56, 0x34, 0xb7],
            vec![1],
        ),
        (
            "pointers",
            "BYTE POINTER a,b",
            "BYTE POINTER($AB789A),BYTE POINTER(0)",
            vec![0x9a, 0x78, 0xab, 0, 0, 0, 0],
            vec![6],
        ),
        (
            "address_size",
            "BYTE a ADDRESS b SIZE c",
            "$12,ADDRESS($834567),SIZE($EFABCD)",
            vec![0x12, 0, 0x67, 0x45, 0x83, 0, 0xcd, 0xab, 0xef],
            vec![1, 5],
        ),
    ]
    .into_iter()
    .map(|(name, parameters, arguments, expected, padding)| Case {
        name,
        parameters: parameters.into(),
        arguments: arguments.into(),
        expected,
        padding,
    })
    .collect();
    cases.push(Case {
        name: "last_displacement",
        parameters: format!(
            "BYTE {}",
            (0..252)
                .map(|i| format!("p{i}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        arguments: vec!["$A5"; 252].join(","),
        expected: [vec![0xa5; 252], vec![0]].concat(),
        padding: vec![252],
    });
    cases
}

fn source(case: &Case, indirect: bool) -> String {
    let (declarations, setup) = match case.name {
        "captured_pointer" => ("VOLATILE ADDRESS input=$7100\n", ""),
        "banked_pointer" => (
            "ADDRESS POINTER input\n",
            "input=ADDRESS POINTER($12FFFE)\n",
        ),
        _ => ("", ""),
    };
    let callback = if indirect {
        format!("LONGCARD FUNC POINTER callback({})\n", case.parameters)
    } else {
        String::new()
    };
    format!(
        "MODULE TEST\nPUBLIC EXTERNAL LONGCARD FUNC Observe({})\n{callback}\
         {declarations}LONGCARD result\nPROC Main()\n{setup}{}result={}({})\nRETURN\nENDMODULE\n",
        case.parameters,
        if indirect { "callback=@Observe\n" } else { "" },
        if indirect { "callback" } else { "Observe" },
        case.arguments,
    )
}

fn observer(bytes: usize) -> Vec<u8> {
    // Independent ABI reader. X addresses the incoming area's first byte;
    // absolute indexed reads also cover the final incoming byte at displacement 255.
    let mut source = String::from("tsc\nclc\nadc #4\ntax\nsep #$20\n.a8\n");
    for i in 0..bytes {
        source.push_str(&format!("lda a:${i:04x},x\nsta ${:06x}\n", RECORD + i));
    }
    source.push_str("ldx #63\nlda #$AA\nclobber: sta 0,x\ndex\nbpl clobber\nrep #$20\n.a16\nlda #$CDEF\nldx #$89AB\nldy #$DEAD\nrtl\nnop\n");
    assemble(&source, OBSERVE)
}

fn compile_case(
    source: &str,
    optimize: bool,
) -> (actionc::compiler::native65816::Compiled, u32, usize) {
    let prepared = prepare(source, optimize);
    let external = prepared
        .mir
        .routines
        .iter()
        .find(|r| r.entry.external_symbol == Some(runtime_symbol_id("TEST.Observe")))
        .unwrap();
    let mut options = layout();
    options.imports.push(AssemblyImport {
        symbol: runtime_symbol_id("TEST.Observe").0,
        signature: external.signature.0,
        abi: abi::generated::ABI_NAME.into(),
        address: OBSERVE,
        size: 0x1000,
        stack_peak: 0,
        checks_stack: true,
        irq_effect: Default::default(),
    });
    let compiled = prepared.compile(&options).unwrap();
    let main = prepared
        .mir
        .routines
        .iter()
        .find(|r| r.name.to_ascii_uppercase().contains("_MAIN_"))
        .unwrap();
    let machine = compiled
        .machine
        .routines
        .iter()
        .find(|r| r.id == main.id)
        .unwrap();
    let (block, index, outgoing) = main
        .blocks
        .iter()
        .find_map(|b| {
            b.ops.iter().enumerate().find_map(|(i, op)| match op {
                Mir65816Op::Call { plan, .. } => {
                    Some((b.id, i, plan.outgoing_bytes.get() as usize))
                }
                _ => None,
            })
        })
        .unwrap();
    let start = compiled.image.entry + machine.code.mir_spans[&(block, index)].start as u32;
    (compiled, start, outgoing)
}

fn reach(h: &mut Harness, pc: u32) {
    assert!(
        h.cpu
            .run_until(
                &mut h.bus,
                100_000,
                |_| Inputs::default(),
                |cpu| cpu.pc() == pc && cpu.is_instruction_boundary()
            )
            .unwrap(),
        "did not reach {pc:06x}"
    );
}

#[test]
fn assembly_observes_complete_arguments_and_zero_padding_in_both_modes() {
    let mut records = vec![];
    for case in cases() {
        let leaf = observer(case.expected.len());
        for indirect in [false, true] {
            // Capturing an indirect target needs a source home above O; the
            // existing d,S limit cannot fit this maximal outgoing area as well.
            if indirect && case.name == "last_displacement" {
                continue;
            }
            let source = source(&case, indirect);
            for optimize in [false, true] {
                let (compiled, start, outgoing) = compile_case(&source, optimize);
                assert_eq!(outgoing, case.expected.len());
                let image =
                    actionc::mir65816::image::Image::from_json(&compiled.image.to_json().unwrap())
                        .unwrap();
                let caller = caller(image.entry);
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.map(OBSERVE, &leaf, false);
                    h.bus.map(0x12fffe, &[0x9a, 0x78, 0xab, 0xe5], false);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&[0x9a, 0x78, 0xab, 0xe5]);
                    reach(&mut h, start);
                    let body_s = usize::from(h.cpu.registers().s);
                    let area = body_s - outgoing + 1..body_s + 1;
                    h.bus.ram[area.clone()].fill(0x5a);
                    let preserved = h.bus.ram[body_s + 1..0x6000].to_vec();
                    let write_start = h.bus.writes.len();
                    reach(&mut h, OBSERVE);
                    assert_eq!(&h.bus.ram[area.clone()], case.expected);
                    assert_eq!(&h.bus.ram[body_s + 1..0x6000], preserved);
                    assert_eq!(usize::from(h.cpu.registers().s), body_s - outgoing - 3);
                    let mut writes = vec![0; outgoing];
                    for &(address, value) in &h.bus.writes[write_start..] {
                        if area.contains(&(address as usize)) {
                            let at = address as usize - area.start;
                            writes[at] += 1;
                            assert_eq!(
                                value, case.expected[at],
                                "each payload/padding write is final"
                            );
                        }
                    }
                    for &pad in &case.padding {
                        assert_eq!(writes[pad], 1);
                    }
                    // Only the middle byte of an admitted private pointer slot
                    // repeats. Padding and neighboring arguments still get one
                    // write; the external capture remains three exact reads.
                    let repeated: &[usize] = match case.name {
                        "pointer" | "captured_pointer" | "banked_pointer" => &[1],
                        "mixed" => &[5],
                        "pointers" => &[1, 4],
                        "address_size" => &[3, 7],
                        _ => &[],
                    };
                    for (at, &count) in writes.iter().enumerate() {
                        assert!(
                            count == 1 || (count == 2 && repeated.contains(&at)),
                            "{} byte {at}: {count} writes, indirect={indirect} optimized={optimize}",
                            case.name
                        );
                    }
                    if !indirect
                        && matches!(case.name, "pointer" | "captured_pointer" | "banked_pointer")
                    {
                        assert_eq!(
                            writes,
                            [1, 1, 1],
                            "the direct pointer probe must push each payload byte once"
                        );
                    }
                    h.run();
                    h.guards(mask);
                    assert_eq!(&h.bus.ram[RECORD..RECORD + outgoing], case.expected);
                    assert_eq!(h.global(&image, "result", 4), 0x89abcdef);
                    let reads: Vec<_> = h
                        .bus
                        .reads
                        .iter()
                        .copied()
                        .filter(|a| {
                            (0x12fffe..0x130002).contains(a) || (0x7100..0x7104).contains(a)
                        })
                        .collect();
                    assert_eq!(
                        reads,
                        match case.name {
                            "captured_pointer" => vec![0x7100, 0x7101, 0x7102],
                            "banked_pointer" => vec![0x12fffe, 0x12ffff, 0x130000],
                            _ => vec![],
                        }
                    );
                    records.push(json!({"case":case.name,"indirect":indirect,"optimize":optimize,"incoming_i":mask,"outgoing":outgoing,"padding":case.padding,"writes":writes,"code_bytes":image.routines.iter().map(|r|r.size).sum::<u32>(),"cycles":h.cpu.cycles()}));
                }
                // Exercise the real source parser with host CRLF, not a text-only comparison.
                let (crlf, _, _) = compile_case(&source.replace('\n', "\r\n"), optimize);
                assert_eq!(crlf.image.to_json().unwrap(), image.to_json().unwrap());
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            Path::new(&dir).join("call-padding-observations.json"),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn captures_and_nested_calls_preserve_source_evaluation_and_caller_storage() {
    let source = "MODULE TEST\n\
        PUBLIC EXTERNAL LONGCARD FUNC Observe(BYTE a CARD b BYTE POINTER p LONGINT c)\n\
        VOLATILE BYTE input=$7000\nLONGCARD result\nCARD calls,retained\n\
        BYTE FUNC Capture() calls==+1 RETURN(input)\n\
        CARD FUNC Mutate() input=$99 RETURN($3456)\n\
        LONGCARD FUNC Forward(CARD value)\nCARD saved\nsaved=value value==+1\n\
          result=Observe(Capture(),Mutate(),@input,LONGINT($BCDEF012))\n\
          retained=saved+value\nRETURN(result)\n\
        PROC Main() result=Forward($1234) RETURN\nENDMODULE\n";
    for optimize in [false, true] {
        let (compiled, _, _) = compile_case(source, optimize);
        let image = &compiled.image;
        for mask in [0, 4] {
            let mut h = Harness::new(image, &caller(image.entry), mask);
            h.bus.map(OBSERVE, &observer(13), false);
            h.bus.ram[0x7000] = 0x12;
            h.run();
            h.guards(mask);
            assert_eq!(
                &h.bus.ram[RECORD..RECORD + 13],
                &[
                    0x12, 0, 0x56, 0x34, 0, 0x70, 0, 0, 0x12, 0xf0, 0xde, 0xbc, 0
                ]
            );
            assert_eq!(h.global(image, "result", 4), 0x89abcdef);
            assert_eq!(h.global(image, "calls", 2), 1);
            assert_eq!(h.global(image, "retained", 2), 0x2469);
            assert_eq!(h.bus.ram[0x7000], 0x99);
        }
    }
}

#[test]
fn mixed_padding_and_indirect_continuations_execute_at_both_o65_placements() {
    use actionc::mir65816::o65::{self as format, profile::Binding};
    use support::o65 as native;
    let case = cases().into_iter().find(|c| c.name == "mixed").unwrap();
    for optimize in [false, true] {
        for indirect in [false, true] {
            let source = source(&case, indirect)
                .replace("LONGCARD result", "BYTE payload\nLONGCARD result")
                .replace("BYTE POINTER($AB789A)", "@payload");
            let bytes = native::compile(
                &source,
                optimize,
                vec![Binding {
                    symbol: runtime_symbol_id("TEST.Observe").0,
                    name: "Observe".into(),
                    stack_peak: 0,
                    checks_stack: true,
                    irq_effect: Default::default(),
                    domains: 3,
                }],
            );
            let contract = format::inspect(&bytes)
                .unwrap()
                .imports
                .iter()
                .find(|i| i.name == "Observe")
                .unwrap()
                .contract
                .clone();
            for variant in 0..2 {
                let provider = OBSERVE + variant as u32 * 0x20000;
                let placement = native::placement(
                    &bytes,
                    variant,
                    vec![
                        native::fault(variant),
                        format::Provider {
                            name: "Observe".into(),
                            address: provider,
                            size: 0x1000,
                            contract: contract.clone(),
                        },
                    ],
                );
                let image = format::relocate(&bytes, &placement).unwrap();
                let mut expected = case.expected.clone();
                expected[4..7]
                    .copy_from_slice(&native::object(&image, "payload").to_le_bytes()[..3]);
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    h.bus.map(provider, &observer(13), false);
                    h.run();
                    h.guards(mask);
                    assert_eq!(&h.bus.ram[RECORD..RECORD + 13], expected);
                    assert_eq!(h.bus.value(native::object(&image, "result"), 4), 0x89abcdef);
                    native::record(
                        &format!("call-padding-{indirect}-{mask}"),
                        optimize,
                        &bytes,
                        &placement,
                        &image,
                        h.cpu.cycles(),
                        None,
                    );
                }
            }
        }
    }
}

#[test]
fn checked_outgoing_extents_fault_before_any_payload_or_transfer_write() {
    use actionc_vm::native65816::Machine;
    for case in cases()
        .into_iter()
        .filter(|c| ["empty", "word", "mixed"].contains(&c.name))
    {
        for optimize in [false, true] {
            for indirect in [false, true] {
                let (compiled, start, outgoing) = compile_case(&source(&case, indirect), optimize);
                let image = &compiled.image;
                let need = outgoing as u16 + if indirect { 6 } else { 3 };
                for (s, floor, ceiling) in [
                    (0x4100u16, 0x4100 - need + 1, 0x5ff0u16),
                    (0x4100, 0x4019, 0x40ff),
                    (need - 1, 0, 0x5ff0),
                ] {
                    let mut h = Harness::new(image, &caller(image.entry), 0);
                    reach(&mut h, start);
                    let mut r = h.cpu.registers();
                    r.s = s;
                    h.cpu = Machine::start_at(r);
                    h.bus.ram[0x2044..0x2046].copy_from_slice(&floor.to_le_bytes());
                    h.bus.ram[0x2046..0x2048].copy_from_slice(&ceiling.to_le_bytes());
                    let before = h.bus.writes.len();
                    reach(&mut h, image.stack_overflow);
                    let r = h.cpu.registers();
                    assert_eq!((r.a, r.x, r.s), (need, s, s));
                    assert_eq!(h.bus.writes.len(), before);
                }
                // Exact guard floor is admitted. Stop at callee entry, before it
                // can add an independent frame reservation of its own.
                let mut h = Harness::new(image, &caller(image.entry), 0);
                h.bus.map(OBSERVE, &observer(13), false);
                reach(&mut h, start);
                let s = h.cpu.registers().s;
                h.bus.ram[0x2044..0x2046].copy_from_slice(&(s - need).to_le_bytes());
                reach(&mut h, OBSERVE);
                assert_eq!(h.cpu.registers().s, s - outgoing as u16 - 3);
                assert_eq!(
                    &h.bus.ram[usize::from(s) - outgoing + 1..=usize::from(s)],
                    case.expected
                );
            }
        }
    }
}
