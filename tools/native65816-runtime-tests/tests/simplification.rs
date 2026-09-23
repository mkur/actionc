//! Explicit exporter: larger compilation probes do not multiply the normal suite.
mod support;
use actionc::mir65816::{emit::proof, image::Image};
use serde_json::json;
use std::{fs, path::PathBuf};
use support::*;

fn ladder(family: &str, size: u16) -> String {
    let statements = (1..=size)
        .map(|i| match family {
            "chain" => format!("n==+{i}\n"),
            "branches" => format!("IF n<32768 THEN n==+{i} ELSE n==-{i} FI\n"),
            _ => unreachable!(),
        })
        .collect::<String>();
    let body = if family == "branches" {
        format!("CARD i\nFOR i=1 TO 4 DO\n{statements}OD\n")
    } else {
        statements
    };
    format!(
        "CARD input,result\nCARD FUNC Work(CARD n)\n{body}RETURN(n)\nPROC Main()\nresult=Work(input)\nRETURN\n"
    )
}

fn execute(image: &Image, family: &str, size: u16) {
    let caller = caller(image.entry);
    for input in [0u16, 13, 32767, 32768, 65535] {
        let mut expected = input;
        for _ in 0..if family == "branches" { 4 } else { 1 } {
            for i in 1..=size {
                expected = if family == "chain" || expected < 32768 {
                    expected.wrapping_add(i)
                } else {
                    expected.wrapping_sub(i)
                };
            }
        }
        for mask in [0, 4] {
            let mut h = Harness::new(image, &caller, mask);
            let address = image
                .data
                .iter()
                .find(|d| d.name == "input")
                .unwrap()
                .address as usize;
            h.bus.ram[address..address + 2].copy_from_slice(&input.to_le_bytes());
            h.run();
            h.guards(mask);
            assert_eq!(h.global(image, "result", 2), u32::from(expected));
        }
    }
}

#[test]
#[ignore = "explicit work-count and size-ladder qualification"]
fn export_work_and_execute_size_ladder() {
    let output =
        PathBuf::from(std::env::var_os("A816_SIMPLIFICATION_DIR").expect("output directory"));
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("layout.json"),
        serde_json::to_string_pretty(&layout()).unwrap(),
    )
    .unwrap();
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    for case in [
        "identity",
        "add",
        "subtract",
        "constant_chain",
        "maximum",
        "wide_shift",
        "loop_rotation",
        "sum_loop",
        "recursive_sum",
        "direct_calls",
        "byte_sum",
        "record_field",
        "unlink",
        "forward_copy",
    ] {
        sources.push((
            case.to_string(),
            fixture(&format!("code_quality/{case}.act")),
            None,
        ));
    }
    for (family, sizes) in [
        ("chain", &[16, 32, 64, 128, 160][..]),
        ("branches", &[4, 8, 16][..]),
    ] {
        for &size in sizes {
            sources.push((
                format!("{family}_{size}"),
                ladder(family, size),
                Some((family, size)),
            ));
        }
    }
    for (case, source, stress) in sources {
        fs::write(output.join(format!("{case}.act")), &source).unwrap();
        for optimized in [false, true] {
            let prepared = prepare(&source, optimized);
            let mir_operations: usize = prepared
                .mir
                .routines
                .iter()
                .flat_map(|r| &r.blocks)
                .map(|b| b.ops.len())
                .sum();
            let (result, work) = proof::measure_work(|| prepared.compile(&layout()));
            let compiled = result.unwrap();
            let serialized = compiled.image.to_json().unwrap();
            let image = Image::from_json(&serialized).unwrap();
            let mode = if optimized { "optimized" } else { "raw" };
            let filename = format!("{case}.{mode}.json");
            fs::write(output.join(&filename), &serialized).unwrap();
            if let Some((family, size)) = stress {
                execute(&image, family, size);
                let crlf = source.replace('\n', "\r\n");
                assert_eq!(
                    prepare(&crlf, optimized)
                        .compile(&layout())
                        .unwrap()
                        .image
                        .to_json()
                        .unwrap(),
                    serialized
                );
                assert!(mir_operations >= usize::from(size), "size ladder collapsed");
            }
            let observations = compiled
                .machine
                .routines
                .iter()
                .flat_map(|r| proof::rewrite_observations(&r.code))
                .collect::<Vec<_>>();
            rows.push(json!({"case":case,"mode":mode,"stress":stress,
                "source":format!("{case}.act"),"image":filename,"mir_operations":mir_operations,
                "code_bytes":image.routines.iter().map(|r| r.size).sum::<u32>(),
                "candidates":observations.len(),"accepted":observations.iter().filter(|o| o.accepted).count(),
                "work":work,"boundary_executions":if stress.is_some() {10} else {0},
                "crlf_equal":stress.is_some()}));
            eprintln!("{case} {mode}: measured and verified");
        }
    }
    fs::write(
        output.join("work.json"),
        serde_json::to_string_pretty(&rows).unwrap(),
    )
    .unwrap();
}
