use super::{Artifact, descriptor, profile::*, wire};
/// Serialize once; all load addresses remain unbound.
pub fn write(artifact: &Artifact) -> Result<Vec<u8>, String> {
    let mut text = artifact.text.clone();
    let descriptor_offset = text.len() as u32;
    text.extend(descriptor::encode(&artifact.profile)?);
    if text.len() >= LIMIT as usize {
        return Err("o65 text exceeds 24 bits".into());
    }
    wire::encode(&wire::File {
        mode: wire::MODE,
        bases: [0; 4],
        lengths: [
            text.len() as u32,
            artifact.data.len() as u32,
            artifact.bss,
            0,
        ],
        stack: 0,
        text,
        data: artifact.data.clone(),
        imports: artifact
            .profile
            .imports
            .iter()
            .map(|i| i.name.clone())
            .collect(),
        relocations: artifact.profile.relocations.clone(),
        exports: vec![
            wire::Export {
                name: ENTRY.into(),
                segment: 2,
                value: artifact.profile.entry,
            },
            wire::Export {
                name: DESCRIPTOR.into(),
                segment: 2,
                value: descriptor_offset,
            },
        ],
    })
}
