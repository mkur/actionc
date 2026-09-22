use super::super::{
    analysis::{homes::HomeByte, sites::SelectedSite},
    effects::Registers,
    selected::{Instruction, Record},
};

/// Only rule constructors in this module tree can author plans. Callers cannot
/// attach custom effects or a proof callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    Identity,
    Adjacent {
        request: SelectedSite,
        temp: super::super::TempId,
        home: super::super::Location,
    },
    #[cfg(test)]
    RemoveNop,
    #[cfg(test)]
    NonDecreasingControl,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Delta {
    pub registers: Registers,
    pub flags: u8,
}
#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct Plan {
    pub(super) rule: Rule,
    pub(super) first: SelectedSite,
    pub(super) last: SelectedSite,
    pub(super) original: Vec<Record>,
    pub(super) replacement: Vec<Instruction>,
    pub(super) removed_definitions: Vec<(HomeByte, SelectedSite)>,
    pub(super) delta: Delta,
}
