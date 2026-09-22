//! Native selected-code analyses. No optimization consumes these facts yet.
#![allow(dead_code)] // Query surface is consumed by the following foundation slices.
pub(super) mod cfg;
mod home_liveness;
pub(super) mod homes;
pub(super) mod sites;

use super::selected::SelectedRoutine;
use crate::analysis::graph::DataflowGraph;
use home_liveness::HomeLiveness;
use homes::{HomeByte, Homes};
use sites::{Node, SelectedSite};
use std::collections::BTreeSet;

pub(super) struct AnalysisSnapshot<'a> {
    selected: &'a SelectedRoutine,
    pub homes: Homes,
    live: HomeLiveness,
}
impl<'a> AnalysisSnapshot<'a> {
    pub fn new(selected: &'a SelectedRoutine) -> Result<Self, String> {
        let homes = Homes::analyze(selected)?;
        let live = HomeLiveness::analyze(selected.cfg(), &homes);
        Ok(Self {
            selected,
            homes,
            live,
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
}

#[cfg(test)]
mod home_tests;
