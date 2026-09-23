//! Closed, checked native rewrite transactions; adjacent temporary LDA is the
//! sole production rule. Identity and synthetic controls remain test consumers.
pub(super) mod context;
pub(super) mod driver;
pub(super) mod pilot;
mod plan;
pub(super) mod rules;

#[cfg(test)]
mod tests;
