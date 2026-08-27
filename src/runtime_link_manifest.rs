use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::linker::{LinkGraph, LinkReason};

pub(crate) const SYS_LINK_MANIFEST_SCHEMA: u32 = 1;
const EMBEDDED_SYS_LINK_MANIFEST: &str = include_str!("../embedded/manifests/sys-link-v1.txt");
const SYS_LINK_SOURCE_KEYS: &[&str] = &[
    "binding:sys-standalone.act",
    "runtime:actionc.act",
    "runtime:sysall.act",
    "runtime:sysblk.act",
    "runtime:sysgr.act",
    "runtime:sysio.act",
    "runtime:syslib.act",
    "runtime:sysmisc.act",
    "runtime:sysreal.act",
    "runtime:sysrealc.act",
    "runtime:sysstr.act",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum RuntimeLinkNode {
    Routine(String),
    Global(String),
    Static(String),
}

impl RuntimeLinkNode {
    pub(crate) fn routine(name: &str) -> Self {
        Self::Routine(canonical_name(name))
    }

    pub(crate) fn global(name: &str) -> Self {
        Self::Global(canonical_name(name))
    }

    pub(crate) fn static_data(name: &str) -> Self {
        Self::Static(canonical_name(name))
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Routine(_) => "routine",
            Self::Global(_) => "global",
            Self::Static(_) => "static",
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Routine(name) | Self::Global(name) | Self::Static(name) => name,
        }
    }

    fn parse(kind: &str, name: &str) -> Result<Self, String> {
        if name.is_empty() || canonical_name(name) != name {
            return Err(format!("runtime-link node name `{name}` is not canonical"));
        }
        match kind {
            "routine" => Ok(Self::Routine(name.to_string())),
            "global" => Ok(Self::Global(name.to_string())),
            "static" => Ok(Self::Static(name.to_string())),
            _ => Err(format!("unknown runtime-link node kind `{kind}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeLinkManifest {
    pub(crate) schema: u32,
    pub(crate) sources: BTreeMap<String, String>,
    pub(crate) graph: LinkGraph<RuntimeLinkNode>,
}

impl RuntimeLinkManifest {
    pub(crate) fn new(graph: LinkGraph<RuntimeLinkNode>) -> Self {
        Self {
            schema: SYS_LINK_MANIFEST_SCHEMA,
            sources: current_source_fingerprints(),
            graph,
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.replace("\r\n", "\n");
        let mut lines = text.lines().enumerate();
        let Some((_, header)) = lines.next() else {
            return Err("embedded SYS link manifest is empty".to_string());
        };
        let header = header.split('\t').collect::<Vec<_>>();
        let [magic, schema] = header.as_slice() else {
            return Err("embedded SYS link manifest has an invalid header".to_string());
        };
        if *magic != "ACTIONC-SYS-LINK-MANIFEST" {
            return Err(format!("unknown SYS link manifest magic `{magic}`"));
        }
        let schema = schema
            .parse::<u32>()
            .map_err(|_| format!("invalid SYS link manifest schema `{schema}`"))?;
        if schema != SYS_LINK_MANIFEST_SCHEMA {
            return Err(format!(
                "unsupported SYS link manifest schema {schema}; expected {SYS_LINK_MANIFEST_SCHEMA}"
            ));
        }

        let mut sources = BTreeMap::new();
        let mut nodes = BTreeSet::new();
        let mut pending_edges = Vec::new();
        for (index, line) in lines {
            let line_number = index + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            match fields.as_slice() {
                ["source", key, digest] => {
                    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(format!(
                            "SYS link manifest line {line_number} has invalid SHA-256 `{digest}`"
                        ));
                    }
                    if sources
                        .insert((*key).to_string(), (*digest).to_string())
                        .is_some()
                    {
                        return Err(format!(
                            "SYS link manifest line {line_number} duplicates source `{key}`"
                        ));
                    }
                }
                ["node", kind, name] => {
                    let node = RuntimeLinkNode::parse(kind, name)?;
                    if !nodes.insert(node.clone()) {
                        return Err(format!(
                            "SYS link manifest line {line_number} duplicates node {kind}:{name}"
                        ));
                    }
                }
                [
                    "edge",
                    source_kind,
                    source_name,
                    target_kind,
                    target_name,
                    reason,
                ] => {
                    let source = RuntimeLinkNode::parse(source_kind, source_name)?;
                    let target = RuntimeLinkNode::parse(target_kind, target_name)?;
                    let reason = LinkReason::from_token(reason).ok_or_else(|| {
                        format!(
                            "SYS link manifest line {line_number} has unknown reason `{reason}`"
                        )
                    })?;
                    pending_edges.push((line_number, source, target, reason));
                }
                _ => {
                    return Err(format!(
                        "SYS link manifest line {line_number} has invalid fields"
                    ));
                }
            }
        }

        let mut graph = LinkGraph::default();
        for node in &nodes {
            graph.add_node(node.clone());
        }
        for (line_number, source, target, reason) in pending_edges {
            if !nodes.contains(&source) || !nodes.contains(&target) {
                return Err(format!(
                    "SYS link manifest line {line_number} references an undeclared node"
                ));
            }
            graph.add_edge(source, target, reason);
        }
        let manifest = Self {
            schema,
            sources,
            graph,
        };
        manifest.validate_sources()?;
        Ok(manifest)
    }

    pub(crate) fn validate_sources(&self) -> Result<(), String> {
        let current = current_source_fingerprints();
        if self.sources == current {
            return Ok(());
        }
        let stale = self
            .sources
            .iter()
            .filter(|(key, digest)| current.get(*key) != Some(*digest))
            .map(|(key, _)| key.as_str())
            .chain(
                current
                    .keys()
                    .filter(|key| !self.sources.contains_key(*key))
                    .map(String::as_str),
            )
            .collect::<BTreeSet<_>>();
        Err(format!(
            "embedded SYS link manifest is stale for: {}",
            stale.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }

    pub(crate) fn render(&self) -> String {
        let mut output = format!("ACTIONC-SYS-LINK-MANIFEST\t{}\n", self.schema);
        for (key, digest) in &self.sources {
            output.push_str(&format!("source\t{key}\t{digest}\n"));
        }
        for node in self.graph.nodes() {
            output.push_str(&format!("node\t{}\t{}\n", node.kind(), node.name()));
        }
        for (source, edge) in self.graph.edges() {
            output.push_str(&format!(
                "edge\t{}\t{}\t{}\t{}\t{}\n",
                source.kind(),
                source.name(),
                edge.target.kind(),
                edge.target.name(),
                edge.reason.token()
            ));
        }
        output
    }
}

pub(crate) fn embedded_sys_link_manifest() -> Result<&'static RuntimeLinkManifest, String> {
    static MANIFEST: OnceLock<Result<RuntimeLinkManifest, String>> = OnceLock::new();
    match MANIFEST.get_or_init(|| RuntimeLinkManifest::parse(EMBEDDED_SYS_LINK_MANIFEST)) {
        Ok(manifest) => Ok(manifest),
        Err(error) => Err(error.clone()),
    }
}

fn current_source_fingerprints() -> BTreeMap<String, String> {
    EmbeddedSourceProvider
        .sources()
        .iter()
        .filter(|source| SYS_LINK_SOURCE_KEYS.contains(&source.canonical_key))
        .map(|source| (source.canonical_key.to_string(), source.sha256.to_string()))
        .collect()
}

fn canonical_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parser_rejects_stale_source_fingerprints() {
        let mut manifest = RuntimeLinkManifest::new(LinkGraph::default());
        let first = manifest
            .sources
            .keys()
            .next()
            .expect("runtime source")
            .clone();
        manifest.sources.insert(first.clone(), "0".repeat(64));
        let error = RuntimeLinkManifest::parse(&manifest.render()).expect_err("stale manifest");
        assert!(error.contains("stale"), "{error}");
        assert!(error.contains(&first), "{error}");
    }

    #[test]
    fn conservative_layout_edges_are_serialized_and_closed_deterministically() {
        let root = RuntimeLinkNode::routine("Opaque");
        let neighbor = RuntimeLinkNode::routine("Neighbor");
        let mut graph = LinkGraph::default();
        graph.add_edge(
            root.clone(),
            neighbor.clone(),
            LinkReason::ConservativeLayout,
        );
        let manifest = RuntimeLinkManifest::new(graph);
        let reparsed = RuntimeLinkManifest::parse(&manifest.render()).expect("parse manifest");
        let selection = reparsed.graph.closure([root]).expect("manifest root");

        assert!(selection.retained.contains(&neighbor));
        assert!(matches!(
            selection.reasons.get(&neighbor),
            Some(crate::linker::LinkRetention::Dependency {
                reason: LinkReason::ConservativeLayout,
                ..
            })
        ));
    }
}
