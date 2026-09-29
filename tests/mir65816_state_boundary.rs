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
//! Native argument pushes replace eight BYTE/word call constructions; symbolic
//! stack execution independently checks payload identity, padding and final S.
//! Shared value tails add one internal REP/join and replace the second tail
//! with BRA in the two-return accumulator and recursion fixtures. Frames and
//! result preparation stay unchanged; labels, fixups and spans are remapped.
//! Equality zero tests remove CMP #0 in optimized sum_loop, forward_copy and
//! recursive_sum, preserving the load's Z;
//! only those instruction bytes and subsequent positions change.
//! Direct incoming-word comparisons omit one unused capture store in maximum
//! (raw/optimized) and optimized recursive_sum. Their loads remain at the
//! comparison; frames and guards are unchanged, and later positions move by
//! two bytes. This reconciles the snapshot with upstream 1d9873cb.
//! Upper-half DP ownership shifts only DP homes and instruction operands by
//! $80. Storage-demand planning leaves these fixtures' emission unchanged.
//! Expression consumers omit adjacent capture/reload pairs for frame stores,
//! comparisons and returns, carry a sole call argument through Y, and return
//! wide XOR results in A/X. Raw sum_loop and both recursive_sum frames shrink;
//! guards remain present with adjusted reservations and incoming offsets.
//! Labels, fixups, sparse home maps and MIR spans follow the selected bytes.
//! Empty-frame entries now remove the 28-byte guard/adjustment prefix. Frames
//! and body bytes are unchanged; later labels, fixups and spans move with it.
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
