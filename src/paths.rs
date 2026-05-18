use crate::store::StoreError;
use std::path::{Path, PathBuf};

fn home_dir() -> Result<PathBuf, StoreError> {
    home::home_dir().ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot determine home directory",
        ))
    })
}

/// Expands a leading `~` or `~/…` (or `~\…` on Windows) using the current user's home
/// directory. Other forms (including `~otheruser/…`) are returned unchanged.
pub(crate) fn expand_tilde(path: impl AsRef<Path>) -> Result<PathBuf, StoreError> {
    let path = path.as_ref();
    let Some(s) = path.to_str() else {
        return Ok(path.to_owned());
    };

    if s == "~" {
        return home_dir();
    }

    if let Some(rest) = tilde_prefix_rest(s) {
        return Ok(home_dir()?.join(rest));
    }

    Ok(path.to_owned())
}

fn tilde_prefix_rest(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix("~/") {
        return Some(rest);
    }
    #[cfg(windows)]
    if let Some(rest) = s.strip_prefix("~\\") {
        return Some(rest);
    }
    None
}

/// `FORAY_HOME` when set (after tilde expansion), otherwise `~/.foray/`.
pub(crate) fn resolve_foray_home() -> Result<PathBuf, StoreError> {
    if let Some(v) = std::env::var("FORAY_HOME").ok().filter(|v| !v.is_empty()) {
        expand_tilde(v)
    } else {
        home_dir().map(|h| h.join(".foray"))
    }
}

#[cfg(test)]
fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Serializes tests that mutate `HOME` / `USERPROFILE` across all modules.
#[cfg(test)]
pub(crate) struct TestHomeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prior_home: Option<String>,
    #[cfg(windows)]
    prior_userprofile: Option<String>,
}

#[cfg(test)]
impl TestHomeGuard {
    /// Point `home::home_dir()` at `dir` for the duration of this guard.
    ///
    /// On Windows, `home::home_dir()` reads `USERPROFILE`, not `HOME`.
    pub(crate) fn set(dir: &Path) -> Self {
        let lock = env_test_lock();

        let prior_home = std::env::var("HOME").ok();
        #[cfg(windows)]
        let prior_userprofile = std::env::var("USERPROFILE").ok();
        let dir_str = dir.as_os_str();
        unsafe {
            std::env::set_var("HOME", dir_str);
            #[cfg(windows)]
            std::env::set_var("USERPROFILE", dir_str);
        }
        Self {
            _lock: lock,
            prior_home,
            #[cfg(windows)]
            prior_userprofile,
        }
    }
}

#[cfg(test)]
impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        restore_env_var("HOME", &self.prior_home);
        #[cfg(windows)]
        restore_env_var("USERPROFILE", &self.prior_userprofile);
    }
}

/// Sets a fake home dir and `FORAY_HOME`; restores both on drop.
#[cfg(test)]
pub(crate) struct TestForayHomeEnv {
    _home: TestHomeGuard,
    prior_foray_home: Option<String>,
}

#[cfg(test)]
impl TestForayHomeEnv {
    pub(crate) fn with(fake_home: &Path, foray_home: impl AsRef<std::ffi::OsStr>) -> Self {
        let home = TestHomeGuard::set(fake_home);
        let prior_foray_home = std::env::var("FORAY_HOME").ok();
        unsafe {
            std::env::set_var("FORAY_HOME", foray_home.as_ref());
        }
        Self {
            _home: home,
            prior_foray_home,
        }
    }
}

#[cfg(test)]
impl Drop for TestForayHomeEnv {
    fn drop(&mut self) {
        restore_env_var("FORAY_HOME", &self.prior_foray_home);
    }
}

#[cfg(test)]
fn restore_env_var(key: &str, prior: &Option<String>) {
    unsafe {
        if let Some(v) = prior {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_alone() {
        let fake_home = tempfile::tempdir().unwrap();
        let _home = TestHomeGuard::set(fake_home.path());
        let home = home_dir().unwrap();
        assert_eq!(expand_tilde("~").unwrap(), home);
    }

    #[test]
    fn expand_tilde_with_slash() {
        let fake_home = tempfile::tempdir().unwrap();
        let _home = TestHomeGuard::set(fake_home.path());
        let home = home_dir().unwrap();
        assert_eq!(
            expand_tilde("~/foo/bar").unwrap(),
            home.join("foo").join("bar")
        );
    }

    #[test]
    fn expand_tilde_leaves_other_forms_unchanged() {
        let fake_home = tempfile::tempdir().unwrap();
        let _home = TestHomeGuard::set(fake_home.path());
        let path = PathBuf::from("/absolute/path");
        assert_eq!(expand_tilde(&path).unwrap(), path);
        assert_eq!(
            expand_tilde("~otheruser/foo").unwrap(),
            PathBuf::from("~otheruser/foo")
        );
    }

    #[test]
    fn resolve_foray_home_from_env() {
        let fake_home = tempfile::tempdir().unwrap();
        let expected = fake_home.path().join("foray-fixture-root");
        let _env = TestForayHomeEnv::with(fake_home.path(), "~/foray-fixture-root");
        assert_eq!(resolve_foray_home().unwrap(), expected);
    }
}
