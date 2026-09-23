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
    let callback = if indirect {
        format!("LONGCARD FUNC POINTER callback({})\n", case.parameters)
    } else {
        String::new()
    };
    format!(
        "MODULE TEST\nPUBLIC EXTERNAL LONGCARD FUNC Observe({})\n{callback}\
         LONGCARD result\nPROC Main()\n{}result={}({})\nRETURN\nENDMODULE\n",
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
                    for &(address, _) in &h.bus.writes[write_start..] {
                        if area.contains(&(address as usize)) {
                            writes[address as usize - area.start] += 1;
                        }
                    }
                    for &pad in &case.padding {
                        assert_eq!(writes[pad], 1);
                    }
                    assert!(writes.iter().all(|&n| n >= 1));
                    h.run();
                    h.guards(mask);
                    assert_eq!(&h.bus.ram[RECORD..RECORD + outgoing], case.expected);
                    assert_eq!(h.global(&image, "result", 4), 0x89abcdef);
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
