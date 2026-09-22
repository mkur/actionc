# Current 65816 memory traffic and register inventory

Measured on 2026-09-22 against scalar-DP compiler `78d0a13` (unchanged compiler
code at `ae1f555`). This completes the quality plan's post-DP measurement step.
It changes only inventory tools and documentation. ABI v1, image v3, o65 profile
v1, stack guards, IRQ reserve and Exec816's compiler pin remain unchanged.

The [full table](benchmarks/65816-register-inventory/tables.md),
[typed facts](benchmarks/65816-register-inventory/facts.json) and
[instruction/vector inventory](benchmarks/65816-register-inventory/inventory.json)
cover all 28 raw/optimized builds: 30 measured worker/helper routine instances
and 28 uncounted Main wrappers. Among the 2,147 counted instruction sites, 177
are word memory LDA sites: 154 stack-relative and 23 DP. These count emitted
loads, including parameter loads; they do not assert redundant reloads.

## Representative results

Cycles run from worker entry through RTL, including guards. Reads/writes below
are **bytes**, including stack arguments and return addresses. DP excludes task
metadata. Both incoming I states and both host VM builds agree.

| Program | Mode | Code bytes | Cycles | Stack peak | Stack R/W | Scratch DP R/W | Executed word LDA stack/DP |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| sum_loop(13) | raw | 146 | 1,587 | 14 | 197 / 246 | 0 / 0 | 69 / 0 |
| sum_loop(13) | optimized | 120 | 1,092 | 6 | 85 / 28 | 80 / 160 | 40 / 27 |
| loop_rotation(13) | raw | 170 | 1,221 | 18 | 133 / 216 | 0 / 0 | 55 / 0 |
| loop_rotation(13) | optimized | 130 | 793 | 8 | 21 / 32 | 102 / 104 | 9 / 41 |

These reproduce the qualified scalar-DP baseline exactly. There is no new code
size, cycle or frame reduction in this slice. For the existing vbcc comparison,
see the [paired baseline table](benchmarks/65816-scalar-dp/after/tables.md).

Optimized sum-loop has 80 mutable-parameter byte reads and 28 writes at S+2:
40 word loads, 13 word stores, and the two-byte entry initialization. The other
five stack reads are the incoming argument and RTL. All 240 scratch-DP byte
accesses belong to private temporary homes. Its 67 executed word memory loads
cost 308 cycles in the existing stream (200 stack + 108 DP). Some loads are
required arithmetic inputs; these costs are not an elimination forecast.

Optimized rotation has 16 staging-byte reads and 16 writes, 16 fixed-local byte
writes, two incoming-argument byte reads, and three RTL reads. All 206 scratch-DP
byte accesses belong to private temporaries. Its 50 word memory loads cost 209
cycles (45 stack + 164 DP). The backedge still rotates two values through one
word of stack staging and separately copies the counter. Its two coalesced
initialization assignments have no physical copy instructions. Logical edge
assignments and same-home assignments are recorded separately from emitted
memory traffic; same-home status alone does not imply every A/N/Z repair can
be omitted.

## Register use and the next candidate

Both optimized loop bodies have **zero explicit X/Y reads or writes**. Their
entry guard writes X with TAX; their nonzero-frame word return uses TAY/TYA to
preserve A across stack release. Reserving either register must respect these
boundaries. Elsewhere in the corpus, wide-shift selection uses LDX/DEX, pointer
accesses use LDY and `[dp],Y`, and call setup uses X for guards and Y to preserve
results. The inventory records these actual instruction effects separately
from conservative MIR operation barriers and JSL ABI clobbers. An unmodelled
operation is not a clobber-free operation; a callee's observed instruction
stream does not grant cross-call residency permission.

The smallest promising follow-up is **X residency for one private unsigned
word loop parameter**, starting with rotation's counter. Dominance identifies
header block 1 and latch block 2. The counter is temp 18 at DP+$22, initialized
to zero, compared with seven, and updated by ADD 1 into temp 12 at DP+$26.
The eight-iteration loop currently executes:

| Counter access | PC | Executions | Existing cycles |
| --- | --- | ---: | ---: |
| Initial STA $22 | $01003B | 1 | 4 |
| Header CMP $22 | $010040 | 9 | 36 |
| Update LDA $22 | $010056 | 8 | 32 |
| Backedge STA $22 | $01006C | 8 | 32 |
| Total | | 26 | 104 |

That is 34 counter-byte reads and 18 writes. The later exit-result store also
uses DP+$22, raising the *physical-home* write total to 20. The inventory retains
all typed home owners so that disjoint lifetimes sharing an address cannot be
mistaken for one value. The update result and input interfere at their closed
ADD operation; the backedge source/destination therefore cannot simply be
assigned one home under the existing rule. X also cannot replace an ADC/CMP
memory operand directly. A follow-up design must select legal instructions,
account for transfers and flag effects, and freeze an exact forecast before
claiming savings. The 104-cycle total is an existing traffic budget only.

Sum-loop's counter is a mutable frame parameter, not a private MIR loop
parameter. Its sole loop parameter is the accumulated sum. Do not infer that a
private-temp residency slice can promote its counter. That requires separate
storage identity, alias and synchronization proofs. Raw casts, wider shifts,
pointer selectors and calling routines provide useful rejection controls.

Keep the initial experiment call-free, with a complete supported-operation
whitelist and an explicit reservation after the guard. Preserve closed-operation
interference, edge A/N/Z requirements and return teardown. Preemption must save
and restore live CPU registers, flags and the task's D/S context using the
existing native contract; call-clobbered DP does not grant interrupt clobber
permission. The [scalar-DP qualification](MIR65816_SCALAR_DP.md) remains the
preemption baseline. This inventory injects no new interrupts and provides no
new register-residency qualification.

## Reproduction and checks

Use the immutable `target/scalar-dp-after` directory with its manifest and both
host execution reports. Reconstruct missing builds with the compiler and runner
from an isolated `78d0a13` checkout following the
[comparison instructions](../tools/compare65816/README.md). Retain all 264 source
records, including the known incorrect optimized vbcc unlink vector 0; the
report is not an exemption for that failure.

From the current checkout:

```sh
A816_COMPARISON_MANIFEST="$PWD/target/scalar-dp-after/manifest.json" \
  A816_MOVEMENT_FACTS=/tmp/register-facts.json \
  cargo test --test mir65816_register_inventory -- --ignored --nocapture
cmp /tmp/register-facts.json docs/benchmarks/65816-register-inventory/facts.json
python3 -B tools/compare65816/inventory_registers.py target/scalar-dp-after \
  --facts /tmp/register-facts.json \
  --output docs/benchmarks/65816-register-inventory --check
python3 -B -m unittest discover -s tools/compare65816 -p 'test_*.py'
```

The new exporter mode reproduces all 28 complete serialized images from current
verified MIR, including all maps and uncounted wrappers, then repeats actual
compilation with LF and CRLF sources (56 additional image comparisons). It adds
current DP operand facts, block PCs and call maps without changing historical
export modes or overwriting the pre-allocation scalar inventory.

The Python inventory verifies source/artifact hashes, exact debug/release report
equality, the complete vector set, and equality with the committed baseline CSV
and its measurement-input hashes. It decodes every final executable byte,
tracks M/X widths under the native disassembler's linear emission contract,
and associates instruction PCs with typed operation spans. Multiplying decoded
access widths by measured execution counts exactly reconciles **all 132 Action
records' stack R/W, scratch-DP R/W and metadata reads**. Indirect pointer reads,
RMW accesses and implicit pushes/return-address transfers are included. Unknown
opcodes, unowned body instructions and unexplained stack accesses fail closed.

Call spans have transient S adjustments: their explicit stack accesses are
reported as `call_span_stack`, without claiming body-home ownership. Pointer
pointee accesses remain separate from the three DP pointer bytes; per-PC
counts do not reveal dynamic pointee identities. Whole-vector reconciliation
checks the measured stack/DP totals. Home traffic keys identify the access's
starting offset and width, rather than redistributing each byte across keys.

Validation: all three exporter modes passed against current images (historical
modes retain schema 1), the snapshot `--check` passed, and all 72 comparison-tool
and five disassembler tests passed, including seven new width, register-effect,
address-space, call-span, traffic-reconciliation and dominance/interference
checks. Execution uses the existing qualified debug/release VM evidence; no VM
rerun or compiler behavior change is needed for this read-only inventory.
