//! Native selected-code analyses consumed through the checked rewrite context.
#![allow(dead_code)] // Additional queries support subsequent measured rules.
pub(super) mod cfg;
mod home_definitions;
mod home_liveness;
pub(super) mod homes;
pub(super) mod machine_liveness;
pub(super) mod sites;

use super::selected::SelectedRoutine;
use crate::analysis::graph::DataflowGraph;
use home_definitions::{Definition, HomeDefinitions, ReadUse, UndefinedRead};
use home_liveness::HomeLiveness;
use homes::{HomeByte, Homes};
use machine_liveness::{MachineLive, MachineLiveness};
use sites::{Node, SelectedSite};
use std::{cell::OnceCell, collections::BTreeSet};

pub(super) struct AnalysisSnapshot<'a> {
    selected: &'a SelectedRoutine,
    pub homes: Homes,
    live: OnceCell<HomeLiveness>,
    definitions: OnceCell<HomeDefinitions>,
    machine: OnceCell<MachineLiveness>,
}
impl<'a> AnalysisSnapshot<'a> {
    pub fn new(selected: &'a SelectedRoutine) -> Result<Self, String> {
        let homes = Homes::analyze(selected)?;
        Ok(Self {
            selected,
            homes,
            live: OnceCell::new(),
            definitions: OnceCell::new(),
            machine: OnceCell::new(),
        })
    }
    // Each cell belongs to this immutable selection only. Edits construct a
    // new snapshot; a missing result never means dead or safe.
    fn live(&self) -> &HomeLiveness {
        self.live
            .get_or_init(|| HomeLiveness::analyze(self.selected.cfg(), &self.homes))
    }
    fn definitions(&self) -> &HomeDefinitions {
        self.definitions
            .get_or_init(|| HomeDefinitions::analyze(self.selected.cfg(), &self.homes))
    }
    fn machine(&self) -> &MachineLiveness {
        self.machine
            .get_or_init(|| MachineLiveness::analyze(self.selected))
    }
    pub fn validate(&self, site: SelectedSite) -> Result<Node, String> {
        let node = self.selected.validate(site)?;
        if !self.selected.cfg().reachable().contains(&node) {
            return Err("unreachable home analysis site".into());
        }
        Ok(node)
    }
    pub fn home_live_before(&self, site: SelectedSite) -> Result<&BTreeSet<HomeByte>, String> {
        let node = self.validate(site)?;
        self.live().before(node)
    }
    pub fn home_live_after(&self, site: SelectedSite) -> Result<&BTreeSet<HomeByte>, String> {
        let node = self.validate(site)?;
        self.live().after(node)
    }
    pub fn site(&self, node: Node) -> Result<SelectedSite, String> {
        self.selected.site(node)
    }
    pub fn uses_of_definition(
        &self,
        home: HomeByte,
        site: SelectedSite,
    ) -> Result<BTreeSet<ReadUse>, String> {
        let store = self.validate(site)?;
        self.definitions()
            .uses_of_definition(&self.homes, Definition { home, store })
    }
    pub fn definition_dead_outside_window(
        &self,
        home: HomeByte,
        store: SelectedSite,
        end: SelectedSite,
    ) -> Result<bool, String> {
        let store = self.validate(store)?;
        let end = self.validate(end)?;
        self.definitions().definition_dead_outside_window(
            self.selected.cfg(),
            &self.homes,
            Definition { home, store },
            end,
        )
    }
    pub fn undefined_private_reads(&self) -> &[UndefinedRead] {
        self.definitions().undefined_private_reads()
    }
    pub fn machine_live_before(&self, site: SelectedSite) -> Result<MachineLive, String> {
        let node = self.validate(site)?;
        self.machine().before(node)
    }
    pub fn machine_live_after(&self, site: SelectedSite) -> Result<MachineLive, String> {
        let node = self.validate(site)?;
        self.machine().after(node)
    }
}

#[cfg(test)]
mod home_tests;

#[cfg(test)]
mod demand_tests;
