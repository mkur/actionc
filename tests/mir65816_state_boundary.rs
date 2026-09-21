//! Frozen before tracker integration: output and nonserialized proof boundaries.
use actionc::{compiler::native65816, includes::ModuleLoadOptions, mir65816};

#[test]
fn native_state_boundary_is_byte_and_metadata_identical() {
    let mut actual = String::new();
    for fixture in [
        "accumulator_forwarding.act",
        "code_quality/sum_loop.act",
        "code_quality/wide_shift.act",
        "code_quality/forward_copy.act",
        "code_quality/recursive_sum.act",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tools/native65816-runtime-tests/tests/fixtures")
            .join(fixture);
        for optimize in [false, true] {
            let mir = native65816::prepare_file(&path, optimize, &ModuleLoadOptions::default())
                .unwrap()
                .mir;
            let machine = mir65816::emit::materialize(&mir).unwrap();
            for r in machine.routines {
                actual.push_str(&format!("{fixture}/{optimize}/{:?}\nframe={:?}\nbytes={:02x?}\nlabels={:?}\nfixups={:?}\nreturns={:?}\nspans={:?}\n", r.id,r.frame,r.code.bytes,r.code.labels,r.code.fixups,r.code.return_fixups,r.code.mir_spans));
            }
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mir65816-state-boundary.txt");
    assert_eq!(
        actual,
        std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
    );
}
