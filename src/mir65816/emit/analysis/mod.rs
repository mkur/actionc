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
use std::collections::BTreeSet;

pub(super) struct AnalysisSnapshot<'a> {
    selected: &'a SelectedRoutine,
    pub homes: Homes,
    live: HomeLiveness,
    definitions: HomeDefinitions,
    machine: MachineLiveness,
}
impl<'a> AnalysisSnapshot<'a> {
    pub fn new(selected: &'a SelectedRoutine) -> Result<Self, String> {
        let homes = Homes::analyze(selected)?;
        let live = HomeLiveness::analyze(selected.cfg(), &homes);
        let definitions = HomeDefinitions::analyze(selected.cfg(), &homes);
        let machine = MachineLiveness::analyze(selected);
        Ok(Self {
            selected,
            homes,
            live,
            definitions,
            machine,
        })
    }
    pub fn validate(&self, site: SelectedSite) -> Result<Node, String> {
        let node = self.selected.validate(site)?;
        if !self.selected.cfg().reachable().contains(&node) {
            return Err("unreachable home analysis site".into());
        }
        Ok(node)
    }
    pub fn home_live_before(&self, site: SelectedSite) -> Result<&BTreeSet<HomeByte>, String> {
        self.live.before(self.validate(site)?)
    }
    pub fn home_live_after(&self, site: SelectedSite) -> Result<&BTreeSet<HomeByte>, String> {
        self.live.after(self.validate(site)?)
    }
    pub fn site(&self, node: Node) -> Result<SelectedSite, String> {
        self.selected.site(node)
    }
    pub fn uses_of_definition(
        &self,
        home: HomeByte,
        site: SelectedSite,
    ) -> Result<BTreeSet<ReadUse>, String> {
        self.definitions.uses_of_definition(
            &self.homes,
            Definition {
                home,
                store: self.validate(site)?,
            },
        )
    }
    pub fn definition_dead_outside_window(
        &self,
        home: HomeByte,
        store: SelectedSite,
        end: SelectedSite,
    ) -> Result<bool, String> {
        self.definitions.definition_dead_outside_window(
            self.selected.cfg(),
            &self.homes,
            Definition {
                home,
                store: self.validate(store)?,
            },
            self.validate(end)?,
        )
    }
    pub fn undefined_private_reads(&self) -> &[UndefinedRead] {
        self.definitions.undefined_private_reads()
    }
    pub fn machine_live_before(&self, site: SelectedSite) -> Result<MachineLive, String> {
        self.machine.before(self.validate(site)?)
    }
    pub fn machine_live_after(&self, site: SelectedSite) -> Result<MachineLive, String> {
        self.machine.after(self.validate(site)?)
    }
}

#[cfg(test)]
mod home_tests;
