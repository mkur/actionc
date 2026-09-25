# Bounded pointer source bindings

MIR65816 selection may omit a nonvolatile, complete three-byte pointer capture
and bind its reads to the unchanged authoritative source home. This is distinct
from incoming/frame word forwarding through A: the binding reuses storage and
publishes neither a temporary store nor an A/N/Z witness.

The private binding records the typed source identity, checked complete home,
captured TempId, defining block/operation and permitted consumer sites. It is
installed only at those sites. Read-side value preparation consults the binding;
all writes still use the allocated temporary home. Allocation, frame extent,
guards and reserved homes are unchanged. Selected instructions, home analysis
and replay see the actual source reads and no fictitious capture definition.

Initially the source is an immutable incoming parameter with no authoritative
frame copy, followed immediately by its sole indirect-load consumer. A closed
routine-wide input count includes address/index operands, terminators and edge
arguments. The consumer retains the same native pointer setup, with both private
word reads inside the checked three-byte source and all d,S bytes in 1..255.
Every external dereference retains its original width, order and volatility.

Parameter admission checks both frame metadata and typed operations. Writes,
address escape, aggregate Copy, partial or displaced/indexed parameter accesses,
volatile access and mutable parameter frame homes reject the source. DP captures
and unsupported consumers retain their complete original capture. Calls and
stores are not consumer candidates. No source-language alias promise, incoming
home allocation exception, cross-call residence or public ABI change is added.
