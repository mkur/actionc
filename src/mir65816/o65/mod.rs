//! Experimental o65 applications. See docs/MIR65816_O65_PROFILE.md.
mod prepare;
pub mod profile;
pub use prepare::{Artifact, prepare};
pub use profile::{Binding, Options};

mod descriptor;
pub mod wire;
mod write;
pub use write::write;

mod read;
pub use read::decode;
mod relocate;
pub use relocate::{Placement, Provider, Region, RelocatedImage, relocate};
