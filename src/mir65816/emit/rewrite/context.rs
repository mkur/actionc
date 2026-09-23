use super::super::{
    analysis::{
        AnalysisSnapshot,
        homes::HomeByte,
        machine_liveness::{ConditionFlag, RegisterLane},
        sites::SelectedSite,
    },
    selected::SelectedRoutine,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir65816::emit) struct Blocker {
    pub reason: String,
    pub site: Option<SelectedSite>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir65816::emit) enum Proof<T> {
    Proven(T),
    Blocked(Blocker),
}
impl<T> Proof<T> {
    pub fn checked(result: Result<T, String>, site: Option<SelectedSite>) -> Self {
        match result {
            Ok(value) => Self::Proven(value),
            Err(reason) => Self::Blocked(Blocker { reason, site }),
        }
    }
    pub fn into_result(self) -> Result<T, Blocker> {
        match self {
            Self::Proven(value) => Ok(value),
            Self::Blocked(reason) => Err(reason),
        }
    }
}

/// Queries share one immutable owner/allocation/generation and reject unknown
/// or unreachable points. A deadness result alone is never a rewrite license.
pub(in crate::mir65816::emit) struct Context<'a> {
    pub selected: &'a SelectedRoutine,
    pub facts: AnalysisSnapshot<'a>,
}
#[allow(dead_code)] // Tested liveness queries are retained for subsequent rules.
impl<'a> Context<'a> {
    pub fn adjacent_load(
        &self,
        site: SelectedSite,
        temp: Option<super::super::TempId>,
        home: Option<super::super::Location>,
        load: &super::super::selected::Instruction,
    ) -> Proof<()> {
        Proof::checked(
            (|| {
                let node = self.facts.validate(site)?;
                let (temp_id, location) = temp.zip(home).ok_or("not a temporary home")?;
                if self.selected.allocation.temps.get(&temp_id) != Some(&location) {
                    return Err("temporary allocation identity mismatch".into());
                }
                let effects = load.effects(self.selected.records()[node.0].before.env);
                let [access] = effects.memory.as_slice() else {
                    return Err("not one private load".into());
                };
                if access.access != super::super::effects::Access::Read {
                    return Err("candidate is not a read".into());
                }
                let range = self
                    .selected
                    .home_contract
                    .as_ref()
                    .ok_or("missing ownership")?
                    .range(access.memory, self.selected.records()[node.0].before.env)
                    .ok_or("unresolved candidate home")?;
                if range.len() != 2
                    || range.iter().any(|h| {
                        !self.facts.homes.info.get(h).is_some_and(|i| {
                            i.private
                                && i.owners.contains(
                                    &super::super::analysis::homes::HomeOwner::Temporary(temp_id),
                                )
                        })
                    })
                {
                    return Err("candidate read is not a private temporary word".into());
                }
                let entry = super::super::replay::prefix(self.selected, node)?;
                entry.prove_adjacent_load(temp, home, load)
            })(),
            Some(site),
        )
    }
    pub fn new(selected: &'a SelectedRoutine) -> Result<Self, String> {
        Ok(Self {
            selected,
            facts: AnalysisSnapshot::new(selected)?,
        })
    }
    pub fn home_dead_after(&self, site: SelectedSite, home: HomeByte) -> Proof<bool> {
        if !self.facts.homes.info.contains_key(&home) {
            return Proof::checked(Err("unknown physical home".into()), Some(site));
        }
        Proof::checked(
            self.facts.home_live_after(site).map(|s| !s.contains(&home)),
            Some(site),
        )
    }
    pub fn definition_dead_outside(
        &self,
        home: HomeByte,
        store: SelectedSite,
        end: SelectedSite,
    ) -> Proof<bool> {
        Proof::checked(
            self.facts.definition_dead_outside_window(home, store, end),
            Some(store),
        )
    }
    pub fn uses_of_definition(&self, home: HomeByte, store: SelectedSite) -> Proof<usize> {
        Proof::checked(
            self.facts.uses_of_definition(home, store).map(|s| s.len()),
            Some(store),
        )
    }
    pub fn register_dead_after(&self, site: SelectedSite, lane: RegisterLane) -> Proof<bool> {
        Proof::checked(
            self.facts
                .machine_live_after(site)
                .map(|s| !s.register_live(lane)),
            Some(site),
        )
    }
    pub fn flag_dead_after(&self, site: SelectedSite, flag: ConditionFlag) -> Proof<bool> {
        Proof::checked(
            self.facts
                .machine_live_after(site)
                .map(|s| !s.flag_live(flag)),
            Some(site),
        )
    }
}
