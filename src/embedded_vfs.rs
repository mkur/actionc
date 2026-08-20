use crate::source::{SourceLoadError, SourceOrigin, SourceProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedSourceKind {
    Module,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedSource {
    pub kind: EmbeddedSourceKind,
    pub canonical_key: &'static str,
    pub virtual_path: &'static str,
    pub display_name: &'static str,
    pub bytes: &'static [u8],
    pub sha256: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/embedded_vfs.rs"));

#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedSourceProvider;

impl EmbeddedSourceProvider {
    pub fn digest(self) -> &'static str {
        VFS_DIGEST
    }

    pub fn sources(self) -> &'static [EmbeddedSource] {
        SOURCES
    }
}

impl SourceProvider for EmbeddedSourceProvider {
    fn read(&self, origin: &SourceOrigin) -> Result<Vec<u8>, SourceLoadError> {
        let Some(virtual_path) = origin.virtual_path() else {
            return Err(SourceLoadError::new(
                "the embedded source provider cannot read a host source",
            ));
        };
        SOURCES
            .iter()
            .find(|source| source.virtual_path == virtual_path)
            .map(|source| source.bytes.to_vec())
            .ok_or_else(|| SourceLoadError::new(format!("embedded source `{origin}` is missing")))
    }

    fn resolve_embedded_module(
        &self,
        canonical_components: &[String],
    ) -> Result<Option<SourceOrigin>, SourceLoadError> {
        let Some((last, parents)) = canonical_components.split_last() else {
            return Ok(None);
        };
        let mut virtual_path = parents.join("/");
        if !virtual_path.is_empty() {
            virtual_path.push('/');
        }
        virtual_path.push_str(last);
        virtual_path.push_str(".act");
        Ok(SOURCES.iter().find_map(|source| {
            (source.kind == EmbeddedSourceKind::Module && source.virtual_path == virtual_path)
                .then(|| SourceOrigin::embedded(source.virtual_path, source.display_name))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod embedded_image {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/build_support/embedded_image.rs"
        ));
    }

    #[test]
    fn sha256_and_aggregate_order_are_deterministic() {
        assert_eq!(
            embedded_image::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let input = |key: &str, byte| embedded_image::SourceInput {
            kind: "Module".to_string(),
            canonical_key: key.to_string(),
            virtual_path: format!("{key}.act"),
            display_name: format!("<embedded:{key}>"),
            bytes: vec![byte],
        };
        let forward = embedded_image::prepare_image(vec![input("a", 1), input("b", 2)]);
        let reverse = embedded_image::prepare_image(vec![input("b", 2), input("a", 1)]);
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .sources
                .iter()
                .map(|source| source.input.canonical_key.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn generated_image_is_sorted_hashed_and_readable_without_host_io() {
        assert_eq!(VFS_DIGEST.len(), 64);
        assert!(
            SOURCES
                .windows(2)
                .all(|pair| pair[0].canonical_key < pair[1].canonical_key)
        );
        assert!(SOURCES.iter().all(|source| source.sha256.len() == 64));
        let runtime = SOURCES
            .iter()
            .find(|source| source.canonical_key == "runtime:syslib.act")
            .expect("SYSLIB runtime source is embedded");
        let origin = SourceOrigin::embedded(runtime.virtual_path, runtime.display_name);
        assert_eq!(
            EmbeddedSourceProvider
                .read(&origin)
                .expect("read embedded source"),
            runtime.bytes
        );
    }
}
