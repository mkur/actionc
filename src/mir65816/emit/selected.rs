//! Admitted native instruction forms; operand encoding size is not CPU width.
#![allow(dead_code)] // Recorded request inputs and immutable queries feed later replay/analysis slices.
use super::super::abi::ResultLocation;
use super::effects::CallContract;
use super::{Label, Target};

macro_rules! instruction_set {
    ($name:ident { $($variant:ident = $byte:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[cfg_attr(not(any(test, feature = "native65816-state-proof")), allow(dead_code))]
        pub(super) enum $name { $($variant),* }
        impl $name { pub(super) fn opcode(self) -> u8 { match self { $(Self::$variant => $byte),* } } }
    };
}
instruction_set!(Implied { Clc=0x18, Sec=0x38, Tcs=0x1b, Tsc=0x3b, Tax=0xaa,
    Tay=0xa8, Tya=0x98, Txa=0x8a, Xba=0xeb, Phk=0x4b, Pha=0x48,
    DecA=0x3a, Rtl=0x6b, Dex=0xca, Inx=0xe8, Iny=0xc8, AslA=0x0a, LsrA=0x4a, Nop=0xea });
instruction_set!(ByteOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    EorImm=0x49, LdaStack=0xa3, StaStack=0x83, AdcStack=0x63, SbcStack=0xe3,
    CmpStack=0xc3, AndStack=0x23, EorStack=0x43, OraStack=0x03, LdaDp=0xa5, StaDp=0x85, LdxDp=0xa6, AdcDp=0x65,
    SbcDp=0xe5, CmpDp=0xc5, AndDp=0x25, OraDp=0x05, EorDp=0x45,
    AslDp=0x06, RolDp=0x26, LsrDp=0x46, RorDp=0x66,
    LdaIndirect=0xa7, StaIndirect=0x87, LdaIndirectY=0xb7, StaIndirectY=0x97,
    Rep=0xc2, Sep=0xe2 });
instruction_set!(WordOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    AndImm=0x29, OraImm=0x09, EorImm=0x49, LdyImm=0xa0, LdxImm=0xa2, CpxImm=0xe0 });
instruction_set!(LongOp { Lda=0xaf, Sta=0x8f });
instruction_set!(ReferenceOp { LdaLong=0xaf, StaLong=0x8f, LdaByte=0xa9, Jsl=0x22, Jml=0x5c });
instruction_set!(Branch { Plus=0x10, Minus=0x30, OverflowClear=0x50, CarryClear=0x90, CarrySet=0xb0, NotEqual=0xd0, Equal=0xf0 });

/// Compound transfers retain their instruction-level phases and ABI summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Instruction {
    Implied(Implied),
    Byte(ByteOp, u8),
    Word(WordOp, u16),
    Long(LongOp, u32),
    Reference(ReferenceOp, Target, u32, Option<u8>),
    Branch(Branch, Label),
    PushReturn(Label),
    /// PHA reserves outgoing payload, rather than an indirect transfer frame.
    ArgumentPush,
    /// PEA reserves exactly two outgoing bytes, independently of M.
    ArgumentPushWord(u16),
    IndirectTransfer(Option<CallContract>),
    NativeCall(Target, CallContract),
    NativeReturn(Option<ResultLocation>),
}

use super::analysis::{
    cfg::SelectedCfg,
    sites::{Identity, Node, SelectedSite},
};
use super::copies::WordHome;
use super::effects::InstructionEffects;
use super::state::{Environment, State65816, Value, Width};
use super::tracked::XContract;
use super::{
    AllocatedFrame, BlockId, Code, Location, Mir65816FrameObjectId, ParamId, RoutineId, Slot,
    TempId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Request inputs only. A successful proof result is never a replay capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Mode(Width),
    EstablishBody,
    Barrier,
    PrepareReturnJoin,
    RegisterHome(Location),
    DeclareBlocks(Vec<Label>),
    ProveEntries {
        predecessors: BTreeMap<Label, BTreeMap<Option<Label>, usize>>,
        reachable: BTreeSet<Label>,
    },
    RememberWord(TempId, Location),
    ConsumeWord(Option<TempId>, Option<Location>, Option<WordHome>),
    RememberFrame(Mir65816FrameObjectId, u32, Slot),
    ConsumeFrame(Mir65816FrameObjectId, u32, Slot, u8),
    CaptureIncoming(ParamId, Slot, TempId, Location),
    StoreIncoming(TempId, Location, Slot),
    ProveX(XContract),
    RefreshX,
    LoadX(Option<TempId>, Option<Location>),
    CompareX(TempId, Location, u16),
    IncrementX(TempId, Location, TempId, Location),
    Jump(Label),
    Fallthrough(Label),
    Dispatch(Branch, Label),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub block: BlockId,
    /// Operation index, or ops.len() for the terminator. Fused spans also name
    /// their terminator through SourceEnd; selected sites remain independent.
    pub index: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Boundary {
    pub env: Environment,
    pub stack_a: Option<i64>,
}
impl Boundary {
    pub fn of(state: &State65816) -> Self {
        Self {
            env: state.env,
            stack_a: match state.a {
                Value::StackAddress(s) => Some(s),
                _ => None,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Entry,
    Instruction {
        form: Instruction,
        effects: InstructionEffects,
        continuation: Option<Label>,
    },
    Request(Request),
    EndRequest(Node),
    Allocate(Label),
    Bind(Label),
    SourceStart(Source),
    SourceEnd {
        source: Source,
        fused_terminator: Option<usize>,
    },
    ReturnExit,
    FaultExit,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub action: Action,
    pub parent: Option<Node>,
    pub source: Option<Source>,
    pub encoded: Range<usize>,
    pub before: Boundary,
    pub after: Boundary,
    /// Observation checked against a freshly recomputed consume decision.
    /// Never an input that grants permission during replay.
    pub decision: Option<bool>,
}
#[derive(Clone, Debug)]
pub(super) struct Recording {
    pub records: Vec<Record>,
    pub parents: Vec<Node>,
    pub source: Option<Source>,
}
impl Default for Recording {
    fn default() -> Self {
        let mut r = Self {
            records: Vec::new(),
            parents: Vec::new(),
            source: None,
        };
        let b = Boundary::of(&State65816::default());
        r.add(Action::Entry, 0..0, b, b);
        r
    }
}
impl Recording {
    pub fn add(
        &mut self,
        action: Action,
        encoded: Range<usize>,
        before: Boundary,
        after: Boundary,
    ) -> Node {
        let node = Node(self.records.len());
        self.records.push(Record {
            action,
            encoded,
            before,
            after,
            parent: self.parents.last().copied(),
            source: self.source,
            decision: None,
        });
        node
    }
}

/// Compiler-owned immutable sequence and graph. Layout may only remap the
/// derived encoded ranges, never identities, actions, effects or graph edges.
#[derive(Clone, Debug)]
pub(super) struct SelectedRoutine {
    identity: Identity,
    pub(super) allocation: AllocatedFrame,
    pub(super) home_contract: Option<super::analysis::homes::HomeContract>,
    records: Vec<Record>,
    cfg: SelectedCfg,
}
impl SelectedRoutine {
    /// Scratch actions are never published without fresh replay/reconciliation.
    pub fn edited(&self, records: Vec<Record>) -> Result<Self, String> {
        let cfg = SelectedCfg::build(&records)?;
        Ok(Self {
            identity: self.identity.checked_next_selection()?,
            allocation: self.allocation.clone(),
            home_contract: self.home_contract.clone(),
            records,
            cfg,
        })
    }
    pub fn new(
        id: RoutineId,
        allocation: &AllocatedFrame,
        home_contract: Option<super::analysis::homes::HomeContract>,
        recording: Recording,
        code: &Code,
    ) -> Result<Self, String> {
        Self::build(
            Identity::fresh(id),
            allocation,
            home_contract,
            recording,
            code,
        )
    }
    pub fn replayed(&self, recording: Recording, code: &Code) -> Result<Self, String> {
        Self::build(
            self.identity,
            &self.allocation,
            self.home_contract.clone(),
            recording,
            code,
        )
    }
    fn build(
        identity: Identity,
        allocation: &AllocatedFrame,
        home_contract: Option<super::analysis::homes::HomeContract>,
        mut recording: Recording,
        code: &Code,
    ) -> Result<Self, String> {
        if !recording.parents.is_empty() || recording.source.is_some() {
            return Err("unfinished selected request/source".into());
        }
        let end = code.bytes.len();
        let b = recording
            .records
            .last()
            .ok_or("missing selected entry")?
            .after;
        recording.add(Action::ReturnExit, end..end, b, b);
        recording.add(Action::FaultExit, end..end, b, b);
        let cfg = SelectedCfg::build(&recording.records)?;
        let result = Self {
            identity,
            allocation: allocation.clone(),
            home_contract,
            records: recording.records,
            cfg,
        };
        result.reconcile(code)?;
        Ok(result)
    }
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    pub fn cfg(&self) -> &SelectedCfg {
        &self.cfg
    }
    pub fn site(&self, node: Node) -> Result<SelectedSite, String> {
        let site = self.identity.site(node);
        self.validate(site)?;
        Ok(site)
    }
    pub fn validate(&self, site: SelectedSite) -> Result<Node, String> {
        self.identity.validate(site, self.records.len())
    }
    pub fn remap(&mut self, map: impl Fn(usize) -> Result<usize, String>) -> Result<(), String> {
        for r in &mut self.records {
            r.encoded = map(r.encoded.start)?..map(r.encoded.end)?;
        }
        Ok(())
    }
    pub fn reconcile(&self, code: &Code) -> Result<(), String> {
        super::analysis::cfg::reconcile(&self.records, code)
    }
}

#[cfg(test)]
#[path = "selected_tests.rs"]
mod tests;
