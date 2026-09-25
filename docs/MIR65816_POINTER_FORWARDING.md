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

The source is an immutable incoming parameter with no authoritative frame copy.
All uses must lie in the defining block within a stable window. A closed
routine-wide input count includes address/index operands, terminators and edge
arguments. Consumers retain the same native pointer setup and operand widths, with private
word reads inside the checked three-byte source and all d,S bytes in 1..255.
Substitution changes only fixed-size stack operands, preserving native selection;
removing the capture cannot make the complete sequence larger.
Every external dereference retains its original width, order and volatility.

Parameter admission checks both frame metadata and typed operations. Writes,
address escape, aggregate Copy, partial or displaced/indexed parameter accesses,
volatile access and mutable parameter frame homes reject the source. DP captures
and unsupported consumers retain their complete original capture. Supported consumers are indirect loads (including indexed loads), address
formation, three-byte comparisons, pointer offsets and three-byte Add/Sub. A
matching native A16/X8 return may use the binding through preparation before
frame teardown. All uses are planned together; a missing or unsupported use
retains the entire capture. Edge-copy schedules remain unsupported.

Calls, stores, aggregate copies and volatile loads end the window, including
when they would be the last consumer. Intervening pure computations and ordinary
loads may clobber registers but cannot change the immutable owned source. No source-language alias promise, incoming
home allocation exception, cross-call residence or public ABI change is added.
