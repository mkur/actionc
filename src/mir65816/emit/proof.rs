//! Opt-in immutable observations for independent machine-code qualification.
//! This feature does not give clients mutable encoder or state access.
pub use super::state::{Value, Width};
use super::{Code, state::State65816, tracked::*};

pub use super::effects::{
    Access as EffectAccess, Control as EffectControl, EffectRecord, InstructionEffects,
    Memory as EffectMemory, Registers as EffectRegisters, env as effect_env,
};
pub use super::tracked::Event;

pub use super::analysis::homes::{HomeAccess, HomeByte, HomeInfo, HomeOwner};
/// Read-only analyses tied to this immutable selection/allocation snapshot.
pub struct HomeAnalysis<'a>(super::analysis::AnalysisSnapshot<'a>);
pub fn home_analysis(code: &Code) -> Result<HomeAnalysis<'_>, String> {
    let selected = code
        .selected
        .as_ref()
        .ok_or("code has no selected routine")?;
    selected.reconcile(code)?;
    Ok(HomeAnalysis(super::analysis::AnalysisSnapshot::new(
        selected,
    )?))
}
impl HomeAnalysis<'_> {
    pub fn homes(&self) -> &std::collections::BTreeMap<HomeByte, HomeInfo> {
        &self.0.homes.info
    }
    pub fn accesses(&self, site: SelectedSite) -> Result<&[HomeAccess], String> {
        Ok(&self.0.homes.accesses[&self.0.validate(site)?])
    }
    pub fn home_live_before(
        &self,
        site: SelectedSite,
    ) -> Result<&std::collections::BTreeSet<HomeByte>, String> {
        self.0.home_live_before(site)
    }
    pub fn home_live_after(
        &self,
        site: SelectedSite,
    ) -> Result<&std::collections::BTreeSet<HomeByte>, String> {
        self.0.home_live_after(site)
    }
}

/// Immutable instruction effects, separate from the historical value snapshots.
pub fn instruction_effects(code: &Code) -> &[EffectRecord] {
    &code.instruction_effects
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeSnapshot {
    pub offset: u16,
    pub direct_page: bool,
    pub width: u8,
    pub generation: u64,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub pc: usize,
    pub event: Event,
    pub anchor: Option<i64>,
    pub transfer_pushes: u8,
    pub native: bool,
    pub decimal: Option<bool>,
    pub dbr: Option<u8>,
    pub current_domain: bool,
    pub irq_preserved: bool,
    pub a: Value,
    pub x: Value,
    pub y: Value,
    pub nz: Value,
    pub carry: Option<bool>,
    pub overflow: Option<bool>,
    pub m: Width,
    pub index: Width,
    pub depth: i64,
    pub homes: Vec<HomeSnapshot>,
}
impl Snapshot {
    pub(super) fn new(pc: usize, s: &State65816) -> Self {
        Self {
            pc,
            event: Event::Instruction,
            anchor: s.env.anchor,
            transfer_pushes: s.env.pushes,
            native: s.env.native,
            decimal: s.env.decimal,
            dbr: s.env.dbr,
            current_domain: s.env.current_domain,
            irq_preserved: s.env.irq_preserved,
            a: s.a,
            x: s.x,
            y: s.y,
            nz: s.nz,
            carry: s.carry,
            overflow: s.overflow,
            m: s.env.m,
            index: s.env.index,
            depth: s.env.depth,
            homes: s
                .homes
                .iter()
                .map(|(&location, h)| HomeSnapshot {
                    offset: location.slot().offset,
                    width: location.slot().width,
                    direct_page: matches!(location, super::Location::DirectPage(_)),
                    generation: h.generation,
                    value: h.value,
                })
                .collect(),
        }
    }
}
/// Fixed probes, independently assembled and executed by the runtime workspace.
pub fn arithmetic_probe(left: u16, right: u16) -> (Code, Vec<Snapshot>) {
    let mut e = TrackedEmitter65816::default();
    e.trace();
    e.a16();
    e.word(WordOp::LdaImm, left);
    e.op(Implied::Clc);
    e.word(WordOp::AdcImm, right);
    e.byte(ByteOp::StaStack, 2);
    e.op(Implied::Tax);
    e.op(Implied::Sec);
    e.word(WordOp::SbcImm, right);
    e.word(WordOp::CmpImm, left);
    e.op(Implied::Tay);
    e.a8();
    e.byte(ByteOp::LdaImm, 0x80);
    e.op(Implied::Xba);
    e.a16();
    e.op(Implied::Tsc);
    e.op(Implied::Clc);
    e.word(WordOp::AdcImm, 0);
    e.op(Implied::Tcs);
    e.finish_traced()
}

#[derive(Clone, Debug)]
pub struct RoutineTrace {
    pub routine: super::RoutineId,
    pub snapshots: Vec<Snapshot>,
}
/// Trace opt-in has no effect on allocation, selection or finalized machine code.
pub fn materialize_with_trace(
    program: &super::Mir65816Program,
) -> Result<(super::MachineProgram, Vec<RoutineTrace>), String> {
    let mut machine = super::materialize_inner(program, true)?;
    let traces = machine
        .routines
        .iter_mut()
        .map(|r| RoutineTrace {
            routine: r.id,
            snapshots: std::mem::take(&mut r.code.state_trace),
        })
        .collect();
    Ok((machine, traces))
}

/// Audited non-control-flow families, in both memory widths. DP contents are
/// deliberately not tracked; private homes and immutable register copies are.
pub fn memory_probe(byte: bool) -> (Code, Vec<Snapshot>) {
    let mut e = TrackedEmitter65816::default();
    e.trace();
    e.register_home(super::Slot {
        offset: 2,
        width: if byte { 1 } else { 2 },
    });
    if byte {
        e.a8();
        e.byte(ByteOp::LdaImm, 0x81);
    } else {
        e.a16();
        e.word(WordOp::LdaImm, 0x8001);
    }
    e.op(Implied::Tax);
    e.byte(ByteOp::StaStack, 2);
    e.byte(ByteOp::LdaStack, 2);
    e.op(Implied::Clc);
    e.byte(ByteOp::AdcStack, 2);
    e.op(Implied::Sec);
    e.byte(ByteOp::SbcStack, 2);
    e.byte(ByteOp::CmpStack, 2);
    e.byte(ByteOp::StaDp, 8);
    e.byte(ByteOp::LdaDp, 8);
    e.op(Implied::Clc);
    e.byte(ByteOp::AdcDp, 8);
    e.op(Implied::Sec);
    e.byte(ByteOp::SbcDp, 8);
    e.byte(ByteOp::CmpDp, 8);
    e.byte(ByteOp::AndDp, 8);
    e.byte(ByteOp::OraDp, 8);
    e.byte(ByteOp::EorDp, 8);
    for op in [ByteOp::AslDp, ByteOp::RolDp, ByteOp::LsrDp, ByteOp::RorDp] {
        e.byte(op, 8);
    }
    e.byte(ByteOp::LdxDp, 8);
    e.word(WordOp::LdyImm, 2);
    e.byte(ByteOp::LdaIndirect, 0);
    e.byte(ByteOp::StaIndirect, 0);
    e.byte(ByteOp::LdaIndirectY, 0);
    e.byte(ByteOp::StaIndirectY, 0);
    e.long(LongOp::Lda, 0x120004).unwrap();
    e.long(LongOp::Sta, 0x120006).unwrap();
    e.op(Implied::Txa);
    e.op(Implied::Tay);
    e.op(Implied::Tya);
    e.op(Implied::Dex);
    e.op(Implied::DecA);
    e.a16();
    e.word(WordOp::AndImm, 0xff);
    e.a8();
    e.byte(ByteOp::EorImm, 0x80);
    e.op(Implied::Clc);
    e.byte(ByteOp::AdcImm, 1);
    e.op(Implied::Sec);
    e.byte(ByteOp::SbcImm, 1);
    e.byte(ByteOp::CmpImm, 0x80);
    e.byte(ByteOp::Sep, 0xdf);
    e.byte(ByteOp::Rep, 0xff);
    e.a8();
    e.op(Implied::Tsc);
    e.op(Implied::Tax);
    e.op(Implied::Tcs);
    e.a16();
    e.op(Implied::Nop);
    e.finish_traced()
}

/// Independent assembly/VM probe for scalar homes in both address spaces.
pub fn scalar_dp_probe(left: u16, right: u16) -> (Code, Vec<Snapshot>) {
    use super::{Location, Slot, copies::WordHome};
    let mut e = TrackedEmitter65816::default();
    e.trace();
    for offset in [32, 34, 36] {
        e.register_home(Location::DirectPage(Slot { offset, width: 2 }));
    }
    e.register_home(Slot {
        offset: 32,
        width: 2,
    });
    e.a16();
    e.word(WordOp::LdaImm, left);
    e.byte(ByteOp::StaDp, 32);
    e.word(WordOp::LdaImm, right);
    e.byte(ByteOp::StaStack, 32);
    e.byte(ByteOp::LdaDp, 32);
    e.op(Implied::Clc);
    e.byte(ByteOp::AdcStack, 32);
    e.byte(ByteOp::StaDp, 34);
    let home = Location::DirectPage(Slot {
        offset: 34,
        width: 2,
    });
    e.remember_word(super::TempId(0), home);
    assert!(e.consume_word(
        Some(super::TempId(0)),
        Some(home),
        Some(WordHome::DirectPage(34))
    ));
    e.byte(ByteOp::CmpDp, 34);
    e.op(Implied::Sec);
    e.byte(ByteOp::SbcDp, 32);
    e.byte(ByteOp::StaDp, 36);
    e.a8();
    e.byte(ByteOp::LdaImm, 0xff);
    e.byte(ByteOp::StaDp, 33);
    e.a16();
    e.byte(ByteOp::LdaDp, 34);
    e.byte(ByteOp::CmpDp, 36);
    e.op(Implied::Nop);
    let code = e.finish();
    let trace = code.state_trace.clone();
    (code, trace)
}

/// CPX has index width even when A is byte sized; transfers retain their own
/// width rules. No production selection is enabled by this probe.
pub fn x_instruction_probe(value: u16, threshold: u16, byte_a: bool) -> (Code, Vec<Snapshot>) {
    let mut e = TrackedEmitter65816::default();
    e.trace();
    e.a16();
    e.word(WordOp::LdaImm, value);
    e.op(Implied::Tax);
    if byte_a {
        e.a8();
        e.byte(ByteOp::LdaImm, 0x55);
    }
    e.word(WordOp::CpxImm, threshold);
    e.op(Implied::Txa);
    e.op(Implied::Nop);
    e.finish_traced()
}

/// Exercise the shared CPX dispatch/finalizer with both short and long reach.
pub fn x_branch_probe(value: u16, threshold: u16, padding: usize) -> Code {
    let mut e = TrackedEmitter65816::default();
    let yes = e.label();
    let end = e.label();
    e.a16();
    e.word(WordOp::LdaImm, value);
    e.op(Implied::Tax);
    e.word(WordOp::CpxImm, threshold);
    e.dispatch(Branch::CarryClear, yes);
    e.word(WordOp::LdaImm, 0);
    for _ in 0..padding {
        e.op(Implied::Nop);
    }
    e.jump(end);
    e.mark(yes);
    e.a16();
    e.word(WordOp::LdaImm, 1);
    e.mark(end);
    e.op(Implied::Nop);
    super::layout::finalize(e.finish(), true).unwrap()
}

/// Independent width/flag qualification of INX; no reservation is enabled here.
pub fn increment_instruction_probe(
    value: u16,
    byte_a: bool,
    byte_x: bool,
) -> (Code, Vec<Snapshot>) {
    let mut e = TrackedEmitter65816::default();
    e.trace();
    e.a16();
    e.word(WordOp::LdaImm, value);
    e.op(Implied::Tax);
    if byte_x {
        e.byte(ByteOp::Sep, 0x10);
    }
    if byte_a {
        e.a8();
        e.byte(ByteOp::LdaImm, 0x5a);
    } else {
        e.word(WordOp::LdaImm, 0x1234);
    }
    e.op(Implied::Inx);
    e.op(Implied::Txa);
    e.op(Implied::Nop);
    e.finish_traced()
}

pub use super::analysis::sites::SelectedSite;
/// Read-only view of a site. Request names are display metadata, never semantics.
#[derive(Clone, Debug)]
pub struct SelectedObservation {
    pub site: SelectedSite,
    pub ordinal: usize,
    pub kind: &'static str,
    pub request: Option<&'static str>,
    pub parent: Option<SelectedSite>,
    pub encoded: std::ops::Range<usize>,
    pub source: Option<(super::BlockId, usize)>,
    pub fused_terminator: Option<usize>,
    pub control: Option<EffectControl>,
    pub successors: Vec<SelectedSite>,
    pub reachable: bool,
    pub depth_before: i64,
    pub depth_after: i64,
}

pub fn selected_site(code: &Code, site: SelectedSite) -> Result<SelectedObservation, String> {
    use super::selected::{Action, Request};
    use crate::analysis::graph::DataflowGraph;
    let s = code
        .selected
        .as_ref()
        .ok_or("code has no selected routine")?;
    let n = s.validate(site)?;
    let r = &s.records()[n.0];
    let mut request = None;
    let mut control = None;
    let mut fused_terminator = None;
    let kind = match &r.action {
        Action::Entry => "entry",
        Action::Instruction { effects, .. } => {
            control = Some(effects.control);
            "instruction"
        }
        Action::Request(q) => {
            request = Some(match q {
                Request::Mode(_) => "mode",
                Request::EstablishBody => "body-anchor",
                Request::Barrier => "barrier",
                Request::RegisterHome(_) => "home",
                Request::DeclareBlocks(_) => "blocks",
                Request::ProveEntries { .. } => "entry-obligations",
                Request::RememberWord(..) => "remember-word",
                Request::ConsumeWord(..) => "consume-word",
                Request::RememberFrame(..) => "remember-frame",
                Request::ConsumeFrame(..) => "consume-frame",
                Request::CaptureIncoming(..) => "capture-incoming",
                Request::StoreIncoming(..) => "store-incoming",
                Request::ProveX(_) => "x-obligations",
                Request::RefreshX => "refresh-x",
                Request::LoadX(..) => "load-x",
                Request::CompareX(..) => "compare-x",
                Request::IncrementX(..) => "increment-x",
                Request::Jump(_) => "jump",
                Request::Fallthrough(_) => "fallthrough",
                Request::Dispatch(..) => "dispatch",
            });
            "request"
        }
        Action::EndRequest(_) => "end-request",
        Action::Allocate(_) => "allocate-label",
        Action::Bind(_) => "bind-label",
        Action::SourceStart(_) => "source-start",
        Action::SourceEnd {
            fused_terminator: fused,
            ..
        } => {
            fused_terminator = *fused;
            "source-end"
        }
        Action::ReturnExit => "return-exit",
        Action::FaultExit => "fault-exit",
    };
    Ok(SelectedObservation {
        site,
        ordinal: n.0,
        kind,
        request,
        parent: r.parent.map(|p| s.site(p)).transpose()?,
        encoded: r.encoded.clone(),
        source: r.source.map(|s| (s.block, s.index)),
        fused_terminator,
        control,
        successors: s
            .cfg()
            .successors(n)
            .iter()
            .map(|&n| s.site(n))
            .collect::<Result<_, _>>()?,
        reachable: s.cfg().reachable().contains(&n),
        depth_before: r.before.env.depth,
        depth_after: r.after.env.depth,
    })
}

pub fn selected_actions(code: &Code) -> Result<Vec<SelectedObservation>, String> {
    use super::analysis::sites::Node;
    let s = code
        .selected
        .as_ref()
        .ok_or("code has no selected routine")?;
    s.reconcile(code)?;
    (0..s.records().len())
        .map(|n| selected_site(code, s.site(Node(n))?))
        .collect()
}
