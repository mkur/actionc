//! Experimental o65 applications. See docs/MIR65816_O65_PROFILE.md.
mod prepare;
pub mod profile;
pub use prepare::{Artifact, prepare};
pub use profile::{Binding, Options};

pub mod compact;
/// Validate either supported application encoding without inventing debug maps
/// or physical argument descriptions absent from compact files.
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    let file = decode(bytes)?;
    if file
        .exports
        .iter()
        .any(|e| e.name == profile::COMPACT_DESCRIPTOR)
    {
        compact::inspect(bytes).map(|_| ())
    } else {
        inspect(bytes).map(|_| ())
    }
}
mod descriptor;
pub mod wire;
mod write;
pub use write::write;

mod read;
pub use read::decode;
mod relocate;
pub use relocate::{Placement, Provider, Region, RelocatedImage, inspect, relocate};
