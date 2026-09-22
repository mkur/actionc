//! Identities belong to a compilation/allocation/selection, never a byte offset.
use super::super::RoutineId;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Node(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    owner: u64,
    routine: RoutineId,
    allocation: u64,
    generation: u64,
}
impl Identity {
    pub fn checked_next_selection(self) -> Result<Self, String> {
        Ok(Self {
            generation: self
                .generation
                .checked_add(1)
                .ok_or("selection generation overflow")?,
            ..self
        })
    }
    pub fn fresh(routine: RoutineId) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let owner = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("selected snapshot owner overflow");
        Self {
            owner,
            routine,
            allocation: 0,
            generation: 0,
        }
    }
    pub fn next_selection(self) -> Self {
        Self {
            generation: self
                .generation
                .checked_add(1)
                .expect("selection generation overflow"),
            ..self
        }
    }
    pub fn next_allocation(self) -> Self {
        Self {
            allocation: self
                .allocation
                .checked_add(1)
                .expect("allocation generation overflow"),
            generation: 0,
            ..self
        }
    }
    pub fn site(self, node: Node) -> SelectedSite {
        SelectedSite {
            identity: self,
            node,
        }
    }
    pub fn validate(self, site: SelectedSite, nodes: usize) -> Result<Node, String> {
        if site.identity != self {
            return Err("foreign or stale selected site".into());
        }
        if site.node.0 >= nodes {
            return Err("selected site out of bounds".into());
        }
        Ok(site.node)
    }
}

/// Opaque, immutable test observation; only the owning selected snapshot can query it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedSite {
    identity: Identity,
    node: Node,
}
