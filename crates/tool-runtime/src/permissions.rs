use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionScope {
    FileRead { root: String },
    FileWrite { root: String },
    NetworkHost { host: String },
    Named { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionRequirement {
    FileRead { path_pointer: String },
    FileWrite { path_pointer: String },
    NetworkHost { host_pointer: String },
    Named { name: String },
}

impl PermissionRequirement {
    pub(crate) fn validate_definition(&self) -> Result<(), String> {
        match self {
            Self::FileRead { path_pointer }
            | Self::FileWrite { path_pointer }
            | Self::NetworkHost {
                host_pointer: path_pointer,
            } => validate_pointer(path_pointer),
            Self::Named { name } => validate_scope_name(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptProvenance {
    TrustedUser,
    UntrustedContent,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSecurity {
    pub provenance: PromptProvenance,
    #[serde(default)]
    pub injection_signals: Vec<String>,
}

impl PromptSecurity {
    pub fn trusted_user() -> Self {
        Self {
            provenance: PromptProvenance::TrustedUser,
            injection_signals: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionContext {
    pub granted_scopes: Vec<PermissionScope>,
    pub prompt: PromptSecurity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedPermission {
    FileRead { path: String },
    FileWrite { path: String },
    NetworkHost { host: String },
    Named { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionDenial {
    #[error("prompt provenance is not exclusively trusted-user")]
    UntrustedPrompt,
    #[error("prompt contains injection signals")]
    PromptInjection,
    #[error("permission pointer is missing or not a string: {pointer}")]
    InvalidPermissionArgument { pointer: String },
    #[error("path is unsafe: {path}")]
    UnsafePath { path: String },
    #[error("scope configuration is invalid")]
    InvalidScope,
    #[error("scope does not permit the requested capability")]
    ScopeMismatch,
    #[error("network host is unsafe: {host}")]
    UnsafeHost { host: String },
}

pub(crate) fn authorize(
    input: &Value,
    requirements: &[PermissionRequirement],
    context: &PermissionContext,
) -> Result<Vec<ResolvedPermission>, PermissionDenial> {
    if context.prompt.provenance != PromptProvenance::TrustedUser {
        return Err(PermissionDenial::UntrustedPrompt);
    }
    if !context.prompt.injection_signals.is_empty() {
        return Err(PermissionDenial::PromptInjection);
    }

    let mut resolved = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        match requirement {
            PermissionRequirement::FileRead { path_pointer } => {
                let requested = pointer_string(input, path_pointer)?;
                let path = NormalizedPath::parse(requested).map_err(|()| {
                    PermissionDenial::UnsafePath {
                        path: requested.into(),
                    }
                })?;
                let allowed = context.granted_scopes.iter().any(|scope| match scope {
                    PermissionScope::FileRead { root } => {
                        NormalizedPath::parse(root).is_ok_and(|root| root.contains(&path))
                    }
                    _ => false,
                });
                if !allowed {
                    return Err(PermissionDenial::ScopeMismatch);
                }
                resolved.push(ResolvedPermission::FileRead {
                    path: path.as_string(),
                });
            }
            PermissionRequirement::FileWrite { path_pointer } => {
                let requested = pointer_string(input, path_pointer)?;
                let path = NormalizedPath::parse(requested).map_err(|()| {
                    PermissionDenial::UnsafePath {
                        path: requested.into(),
                    }
                })?;
                let allowed = context.granted_scopes.iter().any(|scope| match scope {
                    PermissionScope::FileWrite { root } => {
                        NormalizedPath::parse(root).is_ok_and(|root| root.contains(&path))
                    }
                    _ => false,
                });
                if !allowed {
                    return Err(PermissionDenial::ScopeMismatch);
                }
                resolved.push(ResolvedPermission::FileWrite {
                    path: path.as_string(),
                });
            }
            PermissionRequirement::NetworkHost { host_pointer } => {
                let requested = pointer_string(input, host_pointer)?;
                let host =
                    normalize_host(requested).ok_or_else(|| PermissionDenial::UnsafeHost {
                        host: requested.into(),
                    })?;
                let allowed = context.granted_scopes.iter().any(|scope| match scope {
                    PermissionScope::NetworkHost { host: allowed } => {
                        normalize_host(allowed).is_some_and(|allowed| allowed == host)
                    }
                    _ => false,
                });
                if !allowed {
                    return Err(PermissionDenial::ScopeMismatch);
                }
                resolved.push(ResolvedPermission::NetworkHost { host });
            }
            PermissionRequirement::Named { name } => {
                if validate_scope_name(name).is_err() {
                    return Err(PermissionDenial::InvalidScope);
                }
                let allowed = context.granted_scopes.iter().any(|scope| {
                    matches!(scope, PermissionScope::Named { name: allowed } if allowed == name)
                });
                if !allowed {
                    return Err(PermissionDenial::ScopeMismatch);
                }
                resolved.push(ResolvedPermission::Named { name: name.clone() });
            }
        }
    }
    Ok(resolved)
}

fn pointer_string<'a>(input: &'a Value, pointer: &str) -> Result<&'a str, PermissionDenial> {
    input
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| PermissionDenial::InvalidPermissionArgument {
            pointer: pointer.into(),
        })
}

fn validate_pointer(pointer: &str) -> Result<(), String> {
    if pointer.is_empty() || !pointer.starts_with('/') || pointer.chars().any(char::is_control) {
        Err("JSON pointer must be non-empty, absolute, and free of controls".into())
    } else {
        Ok(())
    }
}

fn validate_scope_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 80
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".:_-".contains(&byte)
        })
    {
        Err("scope names use lowercase ASCII identifiers".into())
    } else {
        Ok(())
    }
}

/// Windows resolves these names to devices in every directory, so a path that
/// ends in one never refers to the file a scope check believes it guards.
/// They are rejected for Unix-shaped paths too: the runtime is Windows-first,
/// and no workspace document legitimately carries one of these names.
fn is_reserved_device_name(stem: &str) -> bool {
    let lowered = stem.to_ascii_lowercase();
    if matches!(lowered.as_str(), "con" | "prn" | "aux" | "nul") {
        return true;
    }
    for prefix in ["com", "lpt"] {
        if let Some(tail) = lowered.strip_prefix(prefix) {
            if matches!(tail, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathPrefix {
    Unix,
    Drive(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedPath {
    prefix: PathPrefix,
    segments: Vec<String>,
}

impl NormalizedPath {
    fn parse(raw: &str) -> Result<Self, ()> {
        if raw.is_empty()
            || raw.chars().any(char::is_control)
            || raw.contains('%')
            || raw.contains('*')
        {
            return Err(());
        }
        let normalized = raw.replace('\\', "/");
        let (prefix, remainder) = if let Some(remainder) = normalized.strip_prefix('/') {
            if remainder.starts_with('/') {
                return Err(());
            }
            (PathPrefix::Unix, remainder)
        } else if normalized.len() >= 3
            && normalized.as_bytes()[0].is_ascii_alphabetic()
            && normalized.as_bytes()[1] == b':'
            && normalized.as_bytes()[2] == b'/'
        {
            (
                PathPrefix::Drive((normalized.as_bytes()[0] as char).to_ascii_uppercase()),
                &normalized[3..],
            )
        } else {
            return Err(());
        };

        let segments: Vec<_> = if remainder.is_empty() {
            Vec::new()
        } else {
            remainder.split('/').map(str::to_owned).collect()
        };
        if segments.iter().any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
        }) {
            return Err(());
        }
        // Windows silently strips trailing dots and spaces, so `draft.md.`
        // and `draft.md ` open the same file as `draft.md` while comparing as
        // different paths here. Device names behave the same way in every
        // directory. Both forms would let the scope check describe something
        // other than the file that is eventually opened.
        if segments.iter().any(|segment| {
            let stripped_by_windows = segment.ends_with('.') || segment.ends_with(' ');
            let stem = segment.split('.').next().unwrap_or(segment.as_str());
            stripped_by_windows || segment.trim().is_empty() || is_reserved_device_name(stem)
        }) {
            return Err(());
        }
        Ok(Self { prefix, segments })
    }

    fn contains(&self, requested: &Self) -> bool {
        if self.prefix != requested.prefix || self.segments.len() > requested.segments.len() {
            return false;
        }
        // A drive-qualified path describes a Windows volume, where
        // `C:/Workspace` and `C:/workspace` are one directory. Comparing them
        // case-sensitively treated them as two, so a granted root quietly
        // stopped covering paths that Windows resolves inside it. Unix-shaped
        // paths stay case-sensitive, because there the two spellings really
        // are two different directories.
        let ignore_case = matches!(self.prefix, PathPrefix::Drive(_));
        for (allowed, actual) in self.segments.iter().zip(&requested.segments) {
            let matched = if ignore_case {
                allowed.to_lowercase() == actual.to_lowercase()
            } else {
                allowed == actual
            };
            if !matched {
                return false;
            }
        }
        true
    }

    fn as_string(&self) -> String {
        let prefix = match self.prefix {
            PathPrefix::Unix => "/".into(),
            PathPrefix::Drive(drive) => format!("{drive}:/"),
        };
        format!("{prefix}{}", self.segments.join("/"))
    }
}

fn normalize_host(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 253 || raw.starts_with('.') || raw.ends_with('.') {
        return None;
    }
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains('*')
        || normalized.contains('/')
        || normalized.contains('\\')
        || normalized.contains(':')
        || normalized.chars().any(char::is_control)
    {
        return None;
    }
    let valid = normalized.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    });
    valid.then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_paths_are_contained_case_insensitively() {
        let root = NormalizedPath::parse("C:/Workspace").unwrap();
        let inside = NormalizedPath::parse("c:\\workspace\\Chapter.md").unwrap();
        let outside = NormalizedPath::parse("C:/Workspace-other/a.md").unwrap();
        assert!(root.contains(&inside));
        assert!(!root.contains(&outside));
    }

    #[test]
    fn unix_paths_stay_case_sensitive() {
        let root = NormalizedPath::parse("/workspace").unwrap();
        let inside = NormalizedPath::parse("/workspace/a.md").unwrap();
        let other = NormalizedPath::parse("/Workspace/a.md").unwrap();
        assert!(root.contains(&inside));
        assert!(!root.contains(&other));
    }

    #[test]
    fn device_names_and_windows_stripped_endings_fail_closed() {
        for raw in [
            "C:/workspace/nul",
            "C:/workspace/COM1.txt",
            "/workspace/aux",
            "C:/workspace/draft.md.",
            "C:/workspace/draft.md ",
            "C:/workspace/ ",
        ] {
            assert!(NormalizedPath::parse(raw).is_err(), "{raw} must be denied");
        }
    }

    #[test]
    fn ordinary_paths_are_still_accepted() {
        for raw in ["C:/workspace/nullify.md", "/workspace/console.md"] {
            assert!(NormalizedPath::parse(raw).is_ok(), "{raw} must be allowed");
        }
    }
}
