//! The protected-path denylist.
//!
//! This check is **lexical only**: it normalizes and compares strings, it
//! never touches the filesystem or resolves symlinks/junctions. That means
//! it cannot see a reparse point swapped in after the check runs. This is
//! intentional and bounded — the action layer re-resolves the real,
//! on-disk path and re-validates it immediately before every move
//! (THREAT_MODEL T9: TOCTOU, path changes between check and move). This
//! module is the first, cheap veto; the second veto in `reclaim-action`,
//! against the resolved path, is the one that closes the TOCTOU gap.

/// A denylist of protected filesystem roots, matched lexically and
/// case-insensitively on path components.
#[derive(Debug, Clone, Default)]
pub struct ProtectedPaths {
    roots: Vec<Vec<String>>,
}

impl ProtectedPaths {
    /// Builds a denylist from an explicit list of root paths.
    pub fn from_roots<I, S>(roots: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut protected = Self::default();
        for root in roots {
            protected.add_root(root.as_ref());
        }
        protected
    }

    /// Builds the standard Windows denylist from well-known environment
    /// variables. Any variable that isn't set is silently skipped, so this
    /// degrades gracefully rather than panicking on an unusual environment.
    ///
    /// Documents/Desktop/Pictures/Videos/Music/Downloads get a *second*,
    /// additive root: whatever [`known_folder_path`] currently resolves for
    /// them (THREAT_MODEL T2). OneDrive Known Folder Move — the default on
    /// most consumer Windows logins — silently relocates those folders
    /// under `%USERPROFILE%\OneDrive\...`; the hardcoded `%USERPROFILE%`
    /// join above stops pointing at the real folder once that's active, so
    /// it alone would leave the user's actual Documents/Desktop
    /// unprotected. This is additive, not a replacement — if
    /// `known_folder_path` fails for any reason, the `%USERPROFILE%` root
    /// added above is still protected (fail safe = protect more, never
    /// less). Resolved fresh on every call, not cached: Known Folder Move
    /// state is per-machine and can change at any time.
    pub fn windows_defaults() -> Self {
        let mut protected = Self::default();

        const ENV_ROOTS: &[(&str, Option<&str>)] = &[
            ("SystemRoot", None),
            ("ProgramFiles", None),
            ("ProgramFiles(x86)", None),
            ("ProgramData", None),
            ("USERPROFILE", Some("Documents")),
            ("USERPROFILE", Some("Desktop")),
            ("USERPROFILE", Some("Pictures")),
            ("USERPROFILE", Some("Videos")),
            ("USERPROFILE", Some("Music")),
            ("USERPROFILE", Some("Downloads")),
            ("APPDATA", None),
            ("LOCALAPPDATA", None),
        ];

        for (var, subdir) in ENV_ROOTS {
            let Ok(value) = std::env::var(var) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            match subdir {
                Some(subdir) => protected.add_root(&format!("{value}\\{subdir}")),
                None => protected.add_root(&value),
            }
        }

        for subdir in [
            "Documents",
            "Desktop",
            "Pictures",
            "Videos",
            "Music",
            "Downloads",
        ] {
            if let Some(resolved) = known_folder_path(subdir) {
                protected.add_root(&resolved);
            }
        }

        protected
    }

    /// Adds a single protected root.
    ///
    /// TODO(cloud): once cloud-provider detection lands, this is where
    /// discovered OneDrive/Dropbox/Google Drive sync roots get added — the
    /// denylist itself doesn't need to change, only who calls this.
    pub fn add_root(&mut self, root: &str) {
        let components = normalize_components(root);
        if !components.is_empty() {
            self.roots.push(components);
        }
    }

    /// Returns true if `path` resolves under any protected root.
    pub fn is_protected(&self, path: &str) -> bool {
        let candidate = normalize_components(path);
        if candidate.is_empty() {
            return false;
        }
        self.roots.iter().any(|root| {
            !root.is_empty() && candidate.len() >= root.len() && candidate[..root.len()] == root[..]
        })
    }
}

/// Resolves a well-known Windows folder's *current* path — e.g. `"Documents"`,
/// `"Desktop"`, `"Downloads"` — via the shell API that actually knows where
/// Windows put it, which correctly follows OneDrive Known Folder Move
/// redirection (`%USERPROFILE%\OneDrive\...`) unlike a hardcoded
/// `%USERPROFILE%` join. Returns `None` for an unrecognized name, on any
/// API failure, or on a non-Windows host — callers must not treat `None` as
/// "nothing to protect here" on its own; see [`ProtectedPaths::windows_defaults`].
#[cfg(windows)]
pub fn known_folder_path(name: &str) -> Option<String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    let folder_id: &GUID = match name {
        "Documents" => &FOLDERID_Documents,
        "Desktop" => &FOLDERID_Desktop,
        "Pictures" => &FOLDERID_Pictures,
        "Videos" => &FOLDERID_Videos,
        "Music" => &FOLDERID_Music,
        "Downloads" => &FOLDERID_Downloads,
        _ => return None,
    };

    // SAFETY: `folder_id` is one of the constant FOLDERID_* GUIDs above,
    // valid for the call. `HANDLE(null)` requests resolution for the
    // current user's token, the standard usage for this API.
    let pwstr =
        unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, HANDLE(std::ptr::null_mut())) }
            .ok()?;
    // SAFETY: `pwstr` is the NUL-terminated UTF-16 string this same call
    // just returned, not yet freed.
    let value = unsafe { pwstr.to_string() }.ok();
    // SAFETY: `pwstr` was allocated by `SHGetKnownFolderPath` and must be
    // freed by the caller exactly once; nothing else references it, and
    // `value` above already copied its contents into an owned `String`.
    unsafe { CoTaskMemFree(Some(pwstr.0.cast())) };
    value
}

/// Non-Windows stub: there is no known-folder concept to resolve. Kept so
/// [`ProtectedPaths::windows_defaults`] doesn't need its own cfg-split.
#[cfg(not(windows))]
pub fn known_folder_path(_name: &str) -> Option<String> {
    None
}

/// Normalizes a path into lowercase, trimmed components for comparison.
///
/// Unifies `/` to `\`, strips `\\?\`, `\\?\UNC\`, and `\\.\` prefixes,
/// lowercases, and trims trailing dots/spaces from each component (the
/// characters Windows silently ignores at the end of a path component,
/// and a classic denylist-evasion trick).
fn normalize_components(path: &str) -> Vec<String> {
    let unified = path.replace('/', "\\");
    let stripped = strip_prefix_markers(&unified);
    let lowered = stripped.to_lowercase();

    lowered
        .split('\\')
        .filter(|c| !c.is_empty())
        .map(trim_trailing_dots_and_spaces)
        .filter(|c| !c.is_empty())
        .collect()
}

fn strip_prefix_markers(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else if let Some(rest) = path.strip_prefix("\\\\.\\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

/// Trims trailing `.` and ` ` from a component, except for the special
/// all-dots components (`.`, `..`) which mean something else entirely and
/// must be left alone.
fn trim_trailing_dots_and_spaces(component: &str) -> String {
    if !component.is_empty() && component.chars().all(|c| c == '.') {
        return component.to_string();
    }
    component.trim_end_matches(['.', ' ']).to_string()
}
