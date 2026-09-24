//! Closed, checked native rewrite transactions. Identity and synthetic controls
//! remain test consumers; each production rule proves its own equivalence.
pub(super) mod context;
pub(super) mod driver;
pub(super) mod pilot;
mod plan;
pub(super) mod rules;
pub(super) mod zero_index;

#[cfg(test)]
mod tests;
