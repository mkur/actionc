//! Deterministic planning dependencies. Selected writes remain authoritative;
//! retained edges additionally protect calls that can reappear on resumption.
use super::*;
use std::collections::BTreeMap;

pub(super) struct CallChains {
    pub order: Vec<RoutineId>,
    pub callees: BTreeMap<RoutineId, BTreeSet<RoutineId>>,
    acyclic: BTreeSet<RoutineId>,
}

impl CallChains {
    pub fn new(input: &MirProgram, selected: &MirProgram) -> Self {
        let mut callees: BTreeMap<RoutineId, BTreeSet<RoutineId>> = BTreeMap::new();
        for routine in input.routines.iter().chain(&selected.routines) {
            let children = callees.entry(routine.id).or_default();
            children.extend(routine.blocks.iter().flat_map(|b| &b.ops).filter_map(|op| match op {
                MirOp::Call { target: MirCallTarget::Routine(target), .. } => Some(*target), _ => None,
            }));
        }
        let mut result = Self { order: Vec::new(), callees, acyclic: BTreeSet::new() };
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();
        for id in result.callees.keys().copied().collect::<Vec<_>>() {
            result.visit(id, &mut visiting, &mut done);
        }
        result
    }

    fn visit(&mut self, id: RoutineId, visiting: &mut BTreeSet<RoutineId>, done: &mut BTreeSet<RoutineId>) -> bool {
        if done.contains(&id) { return self.acyclic.contains(&id); }
        if !self.callees.contains_key(&id) || !visiting.insert(id) { return false; }
        let mut acyclic = true;
        for callee in self.callees[&id].clone() {
            // Visit every child, including independent branches after a cycle.
            acyclic &= self.visit(callee, visiting, done);
        }
        visiting.remove(&id);
        done.insert(id);
        self.order.push(id);
        if acyclic { self.acyclic.insert(id); }
        acyclic
    }

    pub fn supports(&self, target: RoutineId) -> bool { self.acyclic.contains(&target) }

    pub fn inherited(&self, id: RoutineId, proposed: &BTreeMap<RoutineId, BTreeSet<u8>>) -> BTreeSet<u8> {
        self.callees[&id].iter().filter_map(|callee| proposed.get(callee))
            .flat_map(|bytes| bytes.iter().copied()).collect()
    }
}
