use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use super::{EmulatorError, EmulatorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmulatorSelection {
    Auto,
    Atari800,
    Altirra,
}

impl EmulatorSelection {
    pub(crate) fn parse(value: &OsStr) -> Result<Self, EmulatorError> {
        match value.to_str() {
            Some("auto") => Ok(Self::Auto),
            Some("atari800") => Ok(Self::Atari800),
            Some("altirra") => Ok(Self::Altirra),
            _ => Err(EmulatorError::discovery(format!(
                "unknown emulator: {}; expected auto, atari800, or altirra",
                value.to_string_lossy()
            ))),
        }
    }

    fn fixed_kind(self) -> Option<EmulatorKind> {
        match self {
            Self::Auto => None,
            Self::Atari800 => Some(EmulatorKind::Atari800),
            Self::Altirra => Some(EmulatorKind::Altirra),
        }
    }
}

impl Default for EmulatorSelection {
    fn default() -> Self {
        Self::Auto
    }
}

impl fmt::Display for EmulatorSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::Atari800 => "atari800",
            Self::Altirra => "altirra",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveredEmulator {
    pub(crate) kind: EmulatorKind,
    pub(crate) executable: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveryEnvironment {
    windows: bool,
    actionc_emulator: Option<OsString>,
    path_entries: Vec<PathBuf>,
    standard_candidates: Vec<DiscoveredEmulator>,
}

impl DiscoveryEnvironment {
    pub(crate) fn from_process() -> Self {
        let windows = cfg!(windows);
        let actionc_emulator = env::var_os("ACTIONC_EMULATOR");
        let path_entries = env::var_os("PATH")
            .map(|path| env::split_paths(&path).collect())
            .unwrap_or_default();
        let standard_candidates = standard_candidates(windows);

        Self {
            windows,
            actionc_emulator,
            path_entries,
            standard_candidates,
        }
    }
}

pub(crate) fn discover_from_process(
    selection: EmulatorSelection,
    explicit_path: Option<&Path>,
) -> Result<DiscoveredEmulator, EmulatorError> {
    discover(
        selection,
        explicit_path,
        &DiscoveryEnvironment::from_process(),
    )
}

fn discover(
    selection: EmulatorSelection,
    explicit_path: Option<&Path>,
    environment: &DiscoveryEnvironment,
) -> Result<DiscoveredEmulator, EmulatorError> {
    let mut checked = Vec::new();

    if let Some(path) = explicit_path {
        let kind = selection.fixed_kind().or_else(|| infer_kind(path)).ok_or_else(|| {
            EmulatorError::discovery(format!(
                "cannot infer emulator kind from {}; use --emulator atari800 or --emulator altirra",
                path.display()
            ))
        })?;
        return locate_candidate(path, kind, environment, &mut checked)
            .ok_or_else(|| not_found(selection, &checked));
    }

    if let Some(value) = &environment.actionc_emulator {
        let path = Path::new(value);
        let kind = selection.fixed_kind().or_else(|| infer_kind(path)).ok_or_else(|| {
            EmulatorError::discovery(format!(
                "cannot infer emulator kind from ACTIONC_EMULATOR={}; select one with --emulator",
                value.to_string_lossy()
            ))
        })?;
        return locate_candidate(path, kind, environment, &mut checked)
            .ok_or_else(|| not_found(selection, &checked));
    }

    for kind in preference(selection, environment.windows) {
        for name in executable_names(kind, environment.windows) {
            if let Some(found) = find_on_path(OsStr::new(name), kind, environment, &mut checked) {
                return Ok(found);
            }
        }
        for candidate in environment
            .standard_candidates
            .iter()
            .filter(|candidate| candidate.kind == kind)
        {
            checked.push(candidate.executable.clone());
            if is_executable_file(&candidate.executable, environment.windows) {
                return Ok(candidate.clone());
            }
        }
    }

    Err(not_found(selection, &checked))
}

fn locate_candidate(
    path: &Path,
    kind: EmulatorKind,
    environment: &DiscoveryEnvironment,
    checked: &mut Vec<PathBuf>,
) -> Option<DiscoveredEmulator> {
    if has_directory_component(path) {
        checked.push(path.to_path_buf());
        return is_executable_file(path, environment.windows).then(|| DiscoveredEmulator {
            kind,
            executable: path.to_path_buf(),
        });
    }

    find_on_path(path.as_os_str(), kind, environment, checked)
}

fn find_on_path(
    name: &OsStr,
    kind: EmulatorKind,
    environment: &DiscoveryEnvironment,
    checked: &mut Vec<PathBuf>,
) -> Option<DiscoveredEmulator> {
    for directory in &environment.path_entries {
        let candidate = directory.join(name);
        checked.push(candidate.clone());
        if is_executable_file(&candidate, environment.windows) {
            return Some(DiscoveredEmulator {
                kind,
                executable: candidate,
            });
        }
    }
    None
}

fn is_executable_file(path: &Path, windows: bool) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() || windows {
        return metadata.is_file();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn has_directory_component(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    path.is_absolute()
        || path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
        || text.contains('/')
        || text.contains('\\')
}

fn preference(selection: EmulatorSelection, windows: bool) -> Vec<EmulatorKind> {
    match selection {
        EmulatorSelection::Atari800 => vec![EmulatorKind::Atari800],
        EmulatorSelection::Altirra => vec![EmulatorKind::Altirra],
        EmulatorSelection::Auto if windows => {
            vec![EmulatorKind::Altirra, EmulatorKind::Atari800]
        }
        EmulatorSelection::Auto => vec![EmulatorKind::Atari800],
    }
}

fn executable_names(kind: EmulatorKind, windows: bool) -> &'static [&'static str] {
    match (kind, windows) {
        (EmulatorKind::Atari800, true) => &["atari800.exe", "atari800"],
        (EmulatorKind::Atari800, false) => &["atari800"],
        (EmulatorKind::Altirra, true) => &["Altirra64.exe", "Altirra.exe"],
        (EmulatorKind::Altirra, false) => &[],
    }
}

fn infer_kind(path: &Path) -> Option<EmulatorKind> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    match name.as_str() {
        "atari800" | "atari800.exe" => Some(EmulatorKind::Atari800),
        "altirra" | "altirra.exe" | "altirra64.exe" => Some(EmulatorKind::Altirra),
        _ => None,
    }
}

fn standard_candidates(windows: bool) -> Vec<DiscoveredEmulator> {
    let mut candidates = Vec::new();
    if windows {
        for root in [
            env::var_os("ProgramFiles"),
            env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        {
            let root = PathBuf::from(root);
            candidates.push(candidate(
                EmulatorKind::Altirra,
                root.join("Altirra/Altirra64.exe"),
            ));
            candidates.push(candidate(
                EmulatorKind::Altirra,
                root.join("Altirra/Altirra.exe"),
            ));
            candidates.push(candidate(
                EmulatorKind::Atari800,
                root.join("Atari800/atari800.exe"),
            ));
        }
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            let root = PathBuf::from(root).join("Programs/Altirra");
            candidates.push(candidate(EmulatorKind::Altirra, root.join("Altirra64.exe")));
            candidates.push(candidate(EmulatorKind::Altirra, root.join("Altirra.exe")));
        }
    } else {
        for path in [
            "/opt/homebrew/bin/atari800",
            "/usr/local/bin/atari800",
            "/opt/local/bin/atari800",
            "/usr/bin/atari800",
        ] {
            candidates.push(candidate(EmulatorKind::Atari800, path));
        }
    }
    candidates
}

fn candidate(kind: EmulatorKind, executable: impl Into<PathBuf>) -> DiscoveredEmulator {
    DiscoveredEmulator {
        kind,
        executable: executable.into(),
    }
}

fn not_found(selection: EmulatorSelection, checked: &[PathBuf]) -> EmulatorError {
    let expected = match selection {
        EmulatorSelection::Auto => "Atari800 or Altirra",
        EmulatorSelection::Atari800 => "Atari800",
        EmulatorSelection::Altirra => "Altirra",
    };
    let checked = checked
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if checked.is_empty() {
        EmulatorError::discovery(format!(
            "{expected} executable not found; no paths were available"
        ))
    } else {
        EmulatorError::discovery(format!(
            "{expected} executable not found; checked: {checked}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "actionc-emulator-discovery-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create discovery test directory");
            Self(path)
        }

        fn executable(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"test executable").expect("write discovery fixture");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let mut permissions = fs::metadata(&path)
                    .expect("read fixture metadata")
                    .permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(&path, permissions).expect("make fixture executable");
            }
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn environment(windows: bool, path_entries: Vec<PathBuf>) -> DiscoveryEnvironment {
        DiscoveryEnvironment {
            windows,
            actionc_emulator: None,
            path_entries,
            standard_candidates: Vec::new(),
        }
    }

    #[test]
    fn windows_auto_prefers_altirra_even_if_atari800_is_also_present() {
        let directory = TestDir::new();
        directory.executable("atari800.exe");
        let altirra = directory.executable("Altirra64.exe");

        let found = discover(
            EmulatorSelection::Auto,
            None,
            &environment(true, vec![directory.0.clone()]),
        )
        .expect("discover Windows emulator");

        assert_eq!(found.kind, EmulatorKind::Altirra);
        assert_eq!(found.executable, altirra);
    }

    #[test]
    fn non_windows_auto_only_discovers_atari800() {
        let directory = TestDir::new();
        directory.executable("Altirra64.exe");
        let atari800 = directory.executable("atari800");

        let found = discover(
            EmulatorSelection::Auto,
            None,
            &environment(false, vec![directory.0.clone()]),
        )
        .expect("discover Unix emulator");

        assert_eq!(found.kind, EmulatorKind::Atari800);
        assert_eq!(found.executable, atari800);
    }

    #[test]
    fn explicit_path_wins_and_auto_infers_its_adapter() {
        let directory = TestDir::new();
        let explicit = directory.executable("Altirra.exe");
        directory.executable("atari800.exe");

        let found = discover(
            EmulatorSelection::Auto,
            Some(&explicit),
            &environment(true, vec![directory.0.clone()]),
        )
        .expect("discover explicit emulator");

        assert_eq!(found.kind, EmulatorKind::Altirra);
        assert_eq!(found.executable, explicit);
    }

    #[test]
    fn actionc_emulator_precedes_path_search() {
        let configured = TestDir::new();
        let configured_atari800 = configured.executable("atari800");
        let path = TestDir::new();
        path.executable("atari800");
        let mut environment = environment(false, vec![path.0.clone()]);
        environment.actionc_emulator = Some(configured_atari800.clone().into_os_string());

        let found = discover(EmulatorSelection::Auto, None, &environment)
            .expect("discover configured emulator");

        assert_eq!(found.executable, configured_atari800);
    }

    #[test]
    fn unknown_explicit_name_requires_an_adapter_selection() {
        let directory = TestDir::new();
        let explicit = directory.executable("my-emulator.exe");

        let error = discover(
            EmulatorSelection::Auto,
            Some(&explicit),
            &environment(true, Vec::new()),
        )
        .expect_err("unknown executable name should be ambiguous");

        assert!(error.message().contains("use --emulator"));
    }

    #[test]
    fn failure_reports_the_paths_that_were_checked() {
        let directory = TestDir::new();
        let error = discover(
            EmulatorSelection::Atari800,
            None,
            &environment(false, vec![directory.0.clone()]),
        )
        .expect_err("missing emulator should fail");

        assert!(error.message().contains("atari800"));
        assert!(error.message().contains(&directory.0.display().to_string()));
    }
}
