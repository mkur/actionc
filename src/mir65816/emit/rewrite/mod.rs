//! Closed, checked native rewrite transactions. No production rule yet.
#![allow(dead_code)] // The adjacent-load consumer joins in the next slice.
pub(super) mod context;
pub(super) mod driver;
pub(super) mod pilot;
mod plan;
pub(super) mod rules;

#[cfg(test)]
mod tests;
