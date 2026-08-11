use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use atrcopy_rs::{atascii_to_ascii, AtrEntry, AtrImage};

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        print_help();
        process::exit(2);
    };

    let command = args.next();
    let options: Vec<String> = args.collect();
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    let mut image = match AtrImage::from_bytes(data) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("{path}: {err}");
            process::exit(1);
        }
    };

    match command.as_deref() {
        None | Some("list") | Some("ls") => print_directory(&path, &image),
        Some("extract") | Some("x") => {
            let mut all = false;
            let mut out_dir = PathBuf::from(".");
            let mut text_mode = TextMode::Auto;
            let mut wanted = Vec::new();
            let mut iter = options.iter();
            while let Some(option) = iter.next() {
                match option.as_str() {
                    "--all" => all = true,
                    "-o" | "--out-dir" => {
                        let Some(dir) = iter.next() else {
                            eprintln!("{path}: expected directory after {option}");
                            process::exit(2);
                        };
                        out_dir = PathBuf::from(dir);
                    }
                    "--raw-only" => text_mode = TextMode::Never,
                    "--text" => {
                        let Some(mode) = iter.next() else {
                            eprintln!("{path}: expected auto, always, or never after --text");
                            process::exit(2);
                        };
                        text_mode = match TextMode::parse(mode) {
                            Some(mode) => mode,
                            None => {
                                eprintln!("{path}: invalid --text mode `{mode}`");
                                process::exit(2);
                            }
                        };
                    }
                    option if option.starts_with("--text=") => {
                        let mode = option.trim_start_matches("--text=");
                        text_mode = match TextMode::parse(mode) {
                            Some(mode) => mode,
                            None => {
                                eprintln!("{path}: invalid --text mode `{mode}`");
                                process::exit(2);
                            }
                        };
                    }
                    _ => wanted.push(option.clone()),
                }
            }
            if !all && wanted.is_empty() {
                eprintln!("{path}: extract needs --all or at least one Atari filename");
                process::exit(2);
            }
            if let Err(err) = extract_files(&image, all, &wanted, &out_dir, text_mode) {
                eprintln!("{path}: {err}");
                process::exit(1);
            }
        }
        Some("add") | Some("put-copy") => {
            let mut output = None;
            let mut specs = Vec::new();
            let mut iter = options.iter();
            while let Some(option) = iter.next() {
                match option.as_str() {
                    "-o" | "--output" => {
                        let Some(path) = iter.next() else {
                            eprintln!("{path}: expected output ATR after {option}");
                            process::exit(2);
                        };
                        output = Some(PathBuf::from(path));
                    }
                    option if option.starts_with("--output=") => {
                        output = Some(PathBuf::from(option.trim_start_matches("--output=")));
                    }
                    _ => specs.push(option.clone()),
                }
            }
            let Some(output) = output else {
                eprintln!("{path}: add needs -o <output.atr>");
                process::exit(2);
            };
            if output == Path::new(&path) {
                eprintln!("{path}: add output must be a different ATR path");
                process::exit(2);
            }
            if specs.is_empty() {
                eprintln!("{path}: add needs at least one host file");
                process::exit(2);
            }
            let additions = match parse_add_specs(&specs) {
                Ok(additions) => additions,
                Err(err) => {
                    eprintln!("{path}: {err}");
                    process::exit(2);
                }
            };
            let mut targets = HashSet::new();
            for addition in &additions {
                let target = match addition.target_name() {
                    Ok(target) => target,
                    Err(err) => {
                        eprintln!("{path}: {err}");
                        process::exit(1);
                    }
                };
                if !targets.insert(target.clone()) {
                    eprintln!("{path}: duplicate target filename `{target}`");
                    process::exit(1);
                }
                let data = match fs::read(&addition.host_path) {
                    Ok(data) => data,
                    Err(err) => {
                        eprintln!(
                            "{path}: failed to read {}: {err}",
                            addition.host_path.display()
                        );
                        process::exit(1);
                    }
                };
                match image.upsert_file(&target, &data) {
                    Ok(update) => println!(
                        "added {} as {} ({} bytes, {} sectors)",
                        addition.host_path.display(),
                        update.name,
                        update.byte_len,
                        update.sector_count
                    ),
                    Err(err) => {
                        eprintln!("{path}: {err}");
                        process::exit(1);
                    }
                }
            }
            if let Err(err) = fs::write(&output, image.into_bytes()) {
                eprintln!("failed to write {}: {err}", output.display());
                process::exit(1);
            }
            println!("wrote {}", output.display());
        }
        Some("-h") | Some("--help") | Some("help") => print_help(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            print_help();
            process::exit(2);
        }
    }
}

fn print_help() {
    eprintln!("usage:");
    eprintln!("  atrcopy-rs <disk.atr> [list]");
    eprintln!("  atrcopy-rs <disk.atr> extract --all [-o <dir>] [--text=auto|always|never]");
    eprintln!(
        "  atrcopy-rs <disk.atr> extract <NAME.EXT>... [-o <dir>] [--text=auto|always|never]"
    );
    eprintln!("  atrcopy-rs <disk.atr> extract ... --raw-only");
    eprintln!("  atrcopy-rs <disk.atr> add -o <out.atr> <host-file>[=<ATARI.EXT>]...");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    Auto,
    Always,
    Never,
}

impl TextMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct AddSpec {
    host_path: PathBuf,
    atari_name: Option<String>,
}

impl AddSpec {
    fn target_name(&self) -> Result<String, String> {
        let name = if let Some(name) = &self.atari_name {
            name.clone()
        } else {
            self.host_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("{} has no file name", self.host_path.display()))?
                .to_string()
        };
        Ok(normalize_filename(&name))
    }
}

fn print_directory(path: &str, image: &AtrImage) {
    let entries = match image.entries() {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("{path}: {err}");
            process::exit(1);
        }
    };
    println!(
        "{path}: ATR sector_size={} sectors={} files={}",
        image.sector_size(),
        image.sector_count(),
        entries.len()
    );
    for entry in entries {
        let marker = if entry.is_deleted() {
            "D"
        } else if entry.is_directory() {
            "/"
        } else if entry.is_locked() {
            "L"
        } else {
            " "
        };
        println!(
            "{marker} {:>4} {:>4} {}",
            entry.start_sector(),
            entry.sector_count(),
            entry.path()
        );
    }
}

fn extract_files(
    image: &AtrImage,
    all: bool,
    wanted: &[String],
    out_dir: &Path,
    text_mode: TextMode,
) -> Result<(), String> {
    fs::create_dir_all(out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
    let entries = image.entries()?;
    let wanted: Vec<String> = wanted.iter().map(|name| normalize_filename(name)).collect();
    let mut extracted = 0usize;

    for entry in entries
        .iter()
        .filter(|entry| !entry.is_deleted() && !entry.is_directory())
    {
        if !all && !wanted.iter().any(|name| wanted_matches_entry(name, entry)) {
            continue;
        }
        let bytes = image.read_file(entry)?;
        let out_path = out_dir.join(host_path(entry.path()));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        if should_decode_text(entry.path(), text_mode) {
            let raw_path = raw_atascii_path(&out_path);
            fs::write(&raw_path, &bytes)
                .map_err(|err| format!("failed to write {}: {err}", raw_path.display()))?;
            fs::write(&out_path, atascii_to_ascii(&bytes))
                .map_err(|err| format!("failed to write {}: {err}", out_path.display()))?;
            println!(
                "extracted {} -> {} (+ raw {})",
                entry.path(),
                out_path.display(),
                raw_path.display()
            );
        } else {
            fs::write(&out_path, bytes)
                .map_err(|err| format!("failed to write {}: {err}", out_path.display()))?;
            println!("extracted {} -> {}", entry.path(), out_path.display());
        }
        extracted += 1;
    }

    if extracted == 0 {
        return Err("no matching files found".to_string());
    }
    Ok(())
}

fn parse_add_specs(specs: &[String]) -> Result<Vec<AddSpec>, String> {
    let mut additions = Vec::new();
    for spec in specs {
        let (host, atari_name) = match spec.split_once('=') {
            Some((host, atari)) if !host.is_empty() && !atari.is_empty() => {
                (host, Some(atari.to_string()))
            }
            Some(_) => {
                return Err(format!(
                    "invalid add spec `{spec}`; expected host[=ATARI.EXT]"
                ))
            }
            None => (spec.as_str(), None),
        };
        additions.push(AddSpec {
            host_path: PathBuf::from(host),
            atari_name,
        });
    }
    Ok(additions)
}

fn should_decode_text(path: &str, mode: TextMode) -> bool {
    match mode {
        TextMode::Always => true,
        TextMode::Never => false,
        TextMode::Auto => text_like_extension(path),
    }
}

fn text_like_extension(path: &str) -> bool {
    let Some(ext) = path.rsplit('.').next() else {
        return false;
    };
    matches!(
        ext.to_ascii_uppercase().as_str(),
        "ACT" | "ASM" | "DOC" | "TXT" | "EXC" | "HLP" | "LST" | "BAS" | "DEM" | "DM1" | "DM2"
    )
}

fn raw_atascii_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".atascii");
    path.with_file_name(name)
}

fn normalize_filename(name: &str) -> String {
    name.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_uppercase()
}

fn host_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn host_path(path: &str) -> PathBuf {
    path.split('/').map(host_filename).collect()
}

fn wanted_matches_entry(wanted: &str, entry: &AtrEntry) -> bool {
    wanted_matches(wanted, entry.path(), entry.name())
}

fn wanted_matches(wanted: &str, path: &str, plain_name: &str) -> bool {
    let path = normalize_filename(path);
    let plain_name = normalize_filename(plain_name);
    let wanted = wanted.trim_end_matches('/');
    wanted == path || wanted == plain_name || path.starts_with(&format!("{wanted}/"))
}

#[cfg(test)]
mod tests {
    use super::{
        host_path, parse_add_specs, raw_atascii_path, should_decode_text, wanted_matches, TextMode,
    };

    #[test]
    fn parses_add_specs_with_optional_atari_name() {
        let specs =
            parse_add_specs(&["build/TN-C.COM=TN.COM".to_string(), "README".to_string()]).unwrap();

        assert_eq!(
            specs[0].host_path,
            std::path::PathBuf::from("build/TN-C.COM")
        );
        assert_eq!(specs[0].atari_name.as_deref(), Some("TN.COM"));
        assert_eq!(specs[1].host_path, std::path::PathBuf::from("README"));
        assert_eq!(specs[1].atari_name, None);
    }

    #[test]
    fn matches_subdirectory_paths_and_prefixes() {
        assert!(wanted_matches("SRC/LIB.ACT", "SRC/LIB.ACT", "LIB.ACT"));
        assert!(wanted_matches("LIB.ACT", "SRC/LIB.ACT", "LIB.ACT"));
        assert!(wanted_matches("SRC", "SRC/LIB.ACT", "LIB.ACT"));
        assert!(wanted_matches("SRC/", "SRC/LIB.ACT", "LIB.ACT"));
        assert!(!wanted_matches("DOCS", "SRC/LIB.ACT", "LIB.ACT"));
    }

    #[test]
    fn converts_atari_path_to_host_path() {
        assert_eq!(
            host_path("SRC/LIB.ACT"),
            std::path::PathBuf::from("SRC/LIB.ACT")
        );
    }

    #[test]
    fn detects_text_files_in_auto_mode() {
        assert!(should_decode_text("SRC/LIB.ACT", TextMode::Auto));
        assert!(should_decode_text("README.DOC", TextMode::Auto));
        assert!(!should_decode_text("GAME.COM", TextMode::Auto));
        assert!(should_decode_text("GAME.COM", TextMode::Always));
        assert!(!should_decode_text("SRC/LIB.ACT", TextMode::Never));
    }

    #[test]
    fn raw_atascii_sidecar_appends_suffix_after_filename() {
        assert_eq!(
            raw_atascii_path(std::path::Path::new("SRC/LIB.ACT")),
            std::path::PathBuf::from("SRC/LIB.ACT.atascii")
        );
    }
}
