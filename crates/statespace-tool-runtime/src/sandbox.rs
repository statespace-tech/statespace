use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;

const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const DEFAULT_HOME: &str = "/tmp";
const DEFAULT_LANG: &str = "C.UTF-8";
const DEFAULT_LC_ALL: &str = "C.UTF-8";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxEnv {
    path: String,
    home: String,
    lang: String,
    lc_all: String,
}

impl Default for SandboxEnv {
    fn default() -> Self {
        Self {
            path: DEFAULT_PATH.to_string(),
            home: DEFAULT_HOME.to_string(),
            lang: DEFAULT_LANG.to_string(),
            lc_all: DEFAULT_LC_ALL.to_string(),
        }
    }
}

impl SandboxEnv {
    #[must_use]
    pub fn from_host_process() -> Self {
        let mut env = Self::default();
        env.path = merged_path(std::env::var_os("PATH").as_deref());

        env
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn home(&self) -> &str {
        &self.home
    }

    #[must_use]
    pub fn lang(&self) -> &str {
        &self.lang
    }

    #[must_use]
    pub fn lc_all(&self) -> &str {
        &self.lc_all
    }
}

fn merged_path(host_path: Option<&OsStr>) -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    if let Some(host) = host_path {
        for path in std::env::split_paths(host) {
            push_unique_path_entry(&mut entries, &mut seen, &path);
        }
    }

    for segment in DEFAULT_PATH.split(':') {
        let path = Path::new(segment);
        push_unique_path_entry(&mut entries, &mut seen, path);
    }

    if entries.is_empty() {
        return DEFAULT_PATH.to_string();
    }

    entries.join(":")
}

fn push_unique_path_entry(entries: &mut Vec<String>, seen: &mut HashSet<String>, path: &Path) {
    if !path.is_absolute() {
        return;
    }

    let Some(path_str) = path.to_str() else {
        return;
    };

    if path_str.is_empty() {
        return;
    }

    if seen.insert(path_str.to_string()) {
        entries.push(path_str.to_string());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_restricted_runtime_values() {
        let env = SandboxEnv::default();

        assert_eq!(env.path(), DEFAULT_PATH);
        assert_eq!(env.home(), DEFAULT_HOME);
        assert_eq!(env.lang(), DEFAULT_LANG);
        assert_eq!(env.lc_all(), DEFAULT_LC_ALL);
    }

    #[test]
    fn merged_path_adds_defaults_after_host_entries() {
        let merged = merged_path(Some(OsStr::new("/nix/store/bin:/usr/bin:/opt/bin")));

        assert_eq!(
            merged,
            "/nix/store/bin:/usr/bin:/opt/bin:/usr/local/bin:/bin"
        );
    }

    #[test]
    fn merged_path_drops_relative_entries() {
        let merged = merged_path(Some(OsStr::new("./bin:/usr/bin:/tmp/bin")));

        assert_eq!(merged, "/usr/bin:/tmp/bin:/usr/local/bin:/bin");
    }
}
