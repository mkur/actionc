//! Opt-in immutable observations for independent machine-code qualification.
//! This feature does not give clients mutable encoder or state access.
pub use super::state::{Value, Width};
use super::{Code, state::State65816, tracked::*};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub pc: usize,
    pub a: Value,
    pub x: Value,
    pub y: Value,
    pub nz: Value,
    pub carry: Option<bool>,
    pub overflow: Option<bool>,
    pub m: Width,
    pub index: Width,
    pub depth: i32,
    pub homes: Vec<(u16, u8, Value)>,
}
impl Snapshot {
    pub(super) fn new(pc: usize, s: &State65816) -> Self {
        Self {
            pc,
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
                .map(|(&(offset, width), h)| (offset, width, h.value))
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
