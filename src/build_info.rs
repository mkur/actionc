#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildInfo {
    version: &'static str,
    channel: Option<&'static str>,
    commit: Option<&'static str>,
    date: Option<&'static str>,
    target: Option<&'static str>,
    vfs_digest: &'static str,
}

impl BuildInfo {
    pub const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            channel: option_env!("ACTIONC_BUILD_CHANNEL"),
            commit: option_env!("ACTIONC_BUILD_SHA"),
            date: option_env!("ACTIONC_BUILD_DATE"),
            target: option_env!("ACTIONC_BUILD_TARGET"),
            vfs_digest: crate::embedded_vfs::VFS_DIGEST,
        }
    }

    pub fn version_line(self, executable: &str) -> String {
        let mut details = [self.channel, self.commit, self.date, self.target]
            .into_iter()
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        details.push(format!("vfs={}", self.vfs_digest));

        format!("{executable} {} ({})", self.version, details.join("; "))
    }
}

pub fn version_line(executable: &str) -> String {
    BuildInfo::current().version_line(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_build_identity_falls_back_to_the_package_version() {
        let info = BuildInfo {
            version: "1.2.3",
            channel: None,
            commit: None,
            date: None,
            target: None,
            vfs_digest: "abc123",
        };

        assert_eq!(info.version_line("actionc"), "actionc 1.2.3 (vfs=abc123)");
    }

    #[test]
    fn nightly_build_identity_includes_reproducibility_fields() {
        let info = BuildInfo {
            version: "1.2.3",
            channel: Some("nightly"),
            commit: Some("0123456789abcdef"),
            date: Some("2026-08-11T03:17:00Z"),
            target: Some("x86_64-pc-windows-msvc"),
            vfs_digest: "abc123",
        };

        assert_eq!(
            info.version_line("actionc-run"),
            "actionc-run 1.2.3 (nightly; 0123456789abcdef; 2026-08-11T03:17:00Z; x86_64-pc-windows-msvc; vfs=abc123)"
        );
    }

    #[test]
    fn empty_workflow_values_are_not_printed() {
        let info = BuildInfo {
            version: "1.2.3",
            channel: Some(""),
            commit: Some("abc123"),
            date: None,
            target: None,
            vfs_digest: "abc123",
        };

        assert_eq!(
            info.version_line("actionc-emit"),
            "actionc-emit 1.2.3 (abc123; vfs=abc123)"
        );
    }
}
