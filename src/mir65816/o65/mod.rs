//! Experimental o65 applications. See docs/MIR65816_O65_PROFILE.md.
mod prepare;
pub mod profile;
pub use prepare::{Artifact, prepare};
pub use profile::{Binding, Options};
