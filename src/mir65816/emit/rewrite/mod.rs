//! Closed, checked native rewrite transactions; adjacent temporary LDA is the
//! sole production rule. Identity and synthetic controls remain test consumers.
#![allow(dead_code)] // Checked query surface supports subsequent measured rules.
pub(super) mod context;
pub(super) mod driver;
pub(super) mod pilot;
mod plan;
pub(super) mod rules;

#[cfg(test)]
mod tests;
