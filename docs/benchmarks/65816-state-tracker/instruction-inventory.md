# Native emitter instruction boundary

Frozen from `select.rs`, `accumulator.rs` and `code.rs` at `5b7fec1`.
Dynamic selection in `memory`, `binary`, `pointer_step`, word arithmetic and
predicate selection is included. No instruction family is inferred from source
semantics at the encoder boundary.

| Family | Forms currently emitted | Required execution width / effects |
| --- | --- | --- |
| LDA / STA | immediate (LDA), d,S, DP, long numeric/symbolic, [DP], [DP],Y | M; load writes A,NZ; store preserves registers/flags and invalidates aliases |
| LDX / LDY | LDX DP, LDY immediate | X; destination and NZ |
| ADC / SBC | immediate, DP, d,S | M; A,NZCV; decimal clear under ABI |
| AND / ORA / EOR | AND immediate/DP, ORA DP, EOR immediate/DP | M; A,NZ; preserve CV |
| CMP | immediate, DP, d,S | M; NZC; preserves A,V |
| ASL / ROL / LSR / ROR | DP | M; memory,NZC; preserves A,V |
| REP / SEP | immediate status mask (currently $20) | Full mask effect; NZ unaffected by current mask |
| CLC / SEC | implied | C only |
| TAX / TAY / TXA / TYA | implied | Destination width, destination and NZ; partial A lanes conservative |
| TSC / TCS | implied | Always full 16-bit; TSC writes A,NZ, TCS writes S without flags |
| XBA | implied | Exchanges full A lanes; NZ from new low byte independent of M |
| DEC A / DEX | implied | M / X respectively; destination,NZ |
| PHK / PER / PHA | implied / continuation fixup / implied | Push 1 / 2 / M bytes; no register/flag effects |
| BPL / BMI / BCC / BCS / BNE / BEQ | inverse short branch skipping JML | Conditional edge plus fallthrough; preserves state; no synthetic output labels |
| JML | symbolic label or overflow adapter | No fallthrough; checked join or raw fault exit |
| JSL | routine/runtime fixup | Three-byte transfer peak, normal-return ABI, scratch/value/flag barrier |
| RTL | implied | Routine return or separate indirect-transfer phase; three-byte pop |

NOP occurs only in forwarding negative tests. Arbitrary raw opcodes/status
restores are not an accepted selector API. Byte immediates require A8; word
immediates require A16 (LDY uses index width). Labels revoke omission permission
but execution widths follow checked incoming edges, including loop backedges.

The frozen `tests/fixtures/mir65816-state-boundary.txt` covers raw and optimized
forwarding, branches, cyclic edges, byte/word arithmetic, shifts, indirect memory,
calls, recursion and return teardown. It records bytes, labels, symbolic fixups,
PER fixups, frame maps and MIR spans before production integration. Existing
indirect-call and o65 runtime artifact baselines additionally cover PER/resume
and relocated machine bytes.
