//! CLI-owned filesystem sink. Backend tracing never chooses paths or performs I/O.
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};
use serde_json::json;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub(super) struct Artifact {
    directory: PathBuf,
    pub trace: Mir6502Trace,
    source_identity: String,
    compiler_identity: String,
}

impl Artifact {
    pub fn create(directory: PathBuf, options: TraceOptions, source: &str) -> io::Result<Self> {
        // create_dir is intentional: never mix a new journal with old files.
        fs::create_dir(&directory)?;
        let compiler_identity = file_identity(&std::env::current_exe()?)?;
        let result = Self {
            directory,
            trace: Mir6502Trace::new(options),
            source_identity: crate::mir6502::trace::identity(source),
            compiler_identity,
        };
        result.manifest(None)?;
        Ok(result)
    }

    fn manifest(&self, status: Option<&str>) -> io::Result<()> {
        let mut manifest = self.trace.manifest();
        if let Some(status) = status {
            manifest["status"] = json!(status);
        }
        manifest["source_identity"] = json!(self.source_identity);
        manifest["source_identity_scope"] =
            json!("decoded root source; included inputs are represented by backend NIR identity");
        manifest["compiler_identity"] = json!(self.compiler_identity);
        manifest["compiler_version"] = json!(env!("CARGO_PKG_VERSION"));
        manifest["summary_policy"] =
            json!("index bounded to 256 KiB, separate from the detailed journal budget");
        let temp = self.directory.join("manifest.pending");
        fs::write(&temp, serde_json::to_vec_pretty(&manifest)?)?;
        // On Windows rename cannot replace an existing file.
        let destination = self.directory.join("manifest.json");
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(temp, destination)
    }

    pub fn save(&self, status: Option<&str>) -> io::Result<()> {
        let result = self.write_details().and_then(|()| self.manifest(status));
        if result.is_err() {
            let _ = self.manifest(Some("trace-write-failed"));
        }
        result
    }

    fn write_details(&self) -> io::Result<()> {
        let mut events = BufWriter::new(File::create(self.directory.join("events.jsonl"))?);
        self.trace.with_events(|records| -> io::Result<()> {
            for record in records {
                // Only one bounded record is copied at a time; never clone the journal.
                let mut data = record.data.clone();
                if let Some(mir) = data.as_object_mut().and_then(|d|d.remove("mir")) {
                    let name = format!("snapshot-{:06}.mir",record.id);
                    fs::write(self.directory.join(&name),mir.as_str().unwrap_or_default())?;
                    data["mir_file"] = json!(name);
                }
                serde_json::to_writer(&mut events,&json!({"id":record.id,"attempt":record.attempt,"kind":record.kind,"data":data}))?;
                events.write_all(b"\n")?;
            }
            Ok(())
        })?;
        events.flush()?;
        let summary = self.trace.home_summary();
        let mut bounded = Limited {
            bytes: Vec::new(),
            limit: 256 * 1024,
        };
        let summary = if serde_json::to_writer_pretty(&mut bounded, &summary).is_ok() {
            bounded.bytes
        } else {
            serde_json::to_vec_pretty(
                &json!({"status":"index-omitted", "reason":"summary exceeded 256 KiB; use typed events and final snapshot"}),
            )?
        };
        fs::write(self.directory.join("home-summary.json"), summary)
    }
}

struct Limited {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("summary budget"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn file_identity(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut bytes = [0u8; 8192];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        for byte in &bytes[..n] {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

pub(super) fn exit(artifact: Option<&Artifact>, code: i32) -> ! {
    if let Some(artifact) = artifact {
        if let Err(error) = artifact.save(Some("compilation-failed")) {
            eprintln!("failed to write MIR6502 trace: {error}");
        }
    }
    std::process::exit(code)
}

pub(super) fn save_or_exit(artifact: &Artifact) {
    if let Err(error) = artifact.save(None) {
        eprintln!("failed to write MIR6502 trace: {error}");
        std::process::exit(1);
    }
    let manifest = artifact.trace.manifest();
    if manifest["status"] == "filter-error" {
        for error in manifest["filter_errors"].as_array().unwrap() {
            eprintln!("MIR6502 trace: {}", error.as_str().unwrap());
        }
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn write_failure_is_recorded_without_overwriting_an_existing_bundle() {
        let root =
            std::env::temp_dir().join(format!("actionc-trace-writer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let artifact = Artifact::create(root.clone(), TraceOptions::default(), "source").unwrap();
        assert!(Artifact::create(root.clone(), TraceOptions::default(), "other").is_err());
        fs::create_dir(root.join("events.jsonl")).unwrap();
        assert!(artifact.save(None).is_err());
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["status"], "trace-write-failed");
        fs::remove_dir_all(root).unwrap();
    }
}
