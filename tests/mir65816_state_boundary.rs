//! Reviewed emitted output and nonserialized proof boundaries. The original
//! pre-tracker snapshot remains in history; 3a removes MIR-entry REP and
//! 3b removes terminal JMLs to adjacent MIR blocks; 3c shortens only MIR
//! conditional dispatch, remapping every position-bearing contract. Compact
//! staging changes only the reviewed sum-loop frame and its reservation/argument
//! operands. Parameter forwarding removes one raw-recursion LDA and remaps
//! its later labels, fixups and spans. Scalar DP promotion changes admitted
//! homes/operands and removes zero-frame teardown; guards remain present.
//! Native bitwise selection replaces wide_shift's four bytewise XOR lanes with
//! two A16 EORs; only those byte streams and their later MIR span offsets change.
//! Unused call cleanup removes TAY/TYA at the two calls to the void procedure,
//! with the corresponding label, fixup and span remapping.
use actionc::{compiler::native65816, includes::ModuleLoadOptions, mir65816};

#[test]
fn native_emission_boundary_matches_reviewed_snapshot() {
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
