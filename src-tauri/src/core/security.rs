use std::path::Path;

/// Describes why a path was rejected by the jail.
#[derive(Debug, PartialEq)]
pub enum PathJailError {
    AbsoluteUnix,        // Starts with '/'  (e.g. /etc/passwd)
    AbsoluteWindowsDrive, // Starts with [X]:\ or [X]:/ (e.g. C:\Windows)
    DirectoryTraversal,   // Contains '..' segments
    OutsideWorkspace,     // Resolved path is not under workspace root
    WorkspaceInvalid,     // Workspace itself could not be resolved
}

impl std::fmt::Display for PathJailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathJailError::AbsoluteUnix =>
                write!(f, "[PATH_JAIL] SECURITY VIOLATION: ruta Unix absoluta rechazada (empieza con '/'). La IA solo puede trabajar dentro del workspace."),
            PathJailError::AbsoluteWindowsDrive =>
                write!(f, "[PATH_JAIL] SECURITY VIOLATION: ruta de disco absoluta rechazada (ej: C:\\...). La IA solo puede trabajar dentro del workspace."),
            PathJailError::DirectoryTraversal =>
                write!(f, "[PATH_JAIL] SECURITY VIOLATION: intento de Directory Traversal ('..') detectado y bloqueado."),
            PathJailError::OutsideWorkspace =>
                write!(f, "[PATH_JAIL] SECURITY VIOLATION: la ruta resuelta apunta fuera del workspace autorizado."),
            PathJailError::WorkspaceInvalid =>
                write!(f, "[PATH_JAIL] ERROR INTERNO: no se pudo canonicalizar el workspace. Operación abortada por seguridad."),
        }
    }
}

/// Returns `true` if the raw string looks like a Windows drive-absolute path.
/// Matches patterns like: `C:\`, `C:/`, `c:\`, `D:/` (case-insensitive).
fn is_windows_drive_absolute(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let drive = bytes[0];
    let colon = bytes[1];
    let sep   = bytes[2];
    drive.is_ascii_alphabetic() && colon == b':' && (sep == b'\\' || sep == b'/')
}

/// **Path Jail 2.0** — Verifies that `target_path` is strictly inside `workspace_path`.
///
/// Rejection order (fast-path first, before any filesystem I/O):
///   1. Unix-absolute paths  (`/`)
///   2. Windows drive-absolute paths (`C:\`, `D:/`, …)
///   3. Traversal segments   (`..`)
///   4. Canonical containment check
///
/// # Returns
/// - `Ok(())` if the path is allowed.
/// - `Err(PathJailError)` with an explanation if it is blocked.
pub fn check_path(workspace_path: &Path, target_path: &Path) -> Result<(), PathJailError> {
    let raw = target_path.to_string_lossy();

    // ── Fast-path syntactic rejections (no I/O needed) ─────────────────────

    // 1. Unix-style absolute path
    if raw.starts_with('/') {
        return Err(PathJailError::AbsoluteUnix);
    }

    // 2. Windows drive-absolute path
    if is_windows_drive_absolute(&raw) {
        // Only reject if the drive root is NOT a prefix of the workspace itself.
        // This allows paths like  "C:\Users\yemsy\proj\src\main.rs" when
        // workspace == "C:\Users\yemsy\proj" — we defer to the canonicalization
        // check below, which is the authoritative containment test.
        // But if the target jumps to a *different* drive we reject immediately.
        let workspace_str = workspace_path.to_string_lossy();
        let target_drive = raw.chars().next().unwrap_or(' ').to_ascii_uppercase();
        let workspace_drive = workspace_str.chars().next().unwrap_or(' ').to_ascii_uppercase();
        if target_drive != workspace_drive {
            return Err(PathJailError::AbsoluteWindowsDrive);
        }
        // Same drive — fall through to the canonical containment check.
    }

    // 3. Directory traversal
    for component in target_path.components() {
        if component.as_os_str() == ".." {
            return Err(PathJailError::DirectoryTraversal);
        }
    }

    // ── Canonical containment check (authoritative) ────────────────────────

    let workspace_canon = std::fs::canonicalize(workspace_path)
        .map_err(|_| PathJailError::WorkspaceInvalid)?;

    // Build the absolute target path (joining relative paths under workspace)
    let target_absolute = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        workspace_path.join(target_path)
    };

    // If the target already exists on disk, canonicalize it (resolves symlinks)
    if let Ok(target_canon) = std::fs::canonicalize(&target_absolute) {
        if target_canon.starts_with(&workspace_canon) {
            return Ok(());
        } else {
            return Err(PathJailError::OutsideWorkspace);
        }
    }

    // Target does not exist yet (new file). Verify the first existing parent.
    let mut parent = target_absolute.parent();
    while let Some(p) = parent {
        if p.exists() {
            if let Ok(parent_canon) = std::fs::canonicalize(p) {
                if !parent_canon.starts_with(&workspace_canon) {
                    return Err(PathJailError::OutsideWorkspace);
                }
            }
            break;
        }
        parent = p.parent();
    }

    if target_absolute.starts_with(&workspace_canon)
        || target_absolute.starts_with(workspace_path)
    {
        Ok(())
    } else {
        Err(PathJailError::OutsideWorkspace)
    }
}

/// Legacy boolean wrapper kept for backward-compatibility with existing callers.
/// Prefer `check_path` for new code so you can surface the specific error.
pub fn is_path_allowed(workspace_path: &Path, target_path: &Path) -> bool {
    check_path(workspace_path, target_path).is_ok()
}

// ── Unit tests ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ws() -> PathBuf {
        PathBuf::from("C:\\Users\\yemsy\\project")
    }

    #[test]
    fn rejects_unix_absolute() {
        let result = check_path(&ws(), Path::new("/etc/passwd"));
        assert_eq!(result, Err(PathJailError::AbsoluteUnix));
    }

    #[test]
    fn rejects_different_drive() {
        let result = check_path(&ws(), Path::new("D:\\secret\\file.txt"));
        assert_eq!(result, Err(PathJailError::AbsoluteWindowsDrive));
    }

    #[test]
    fn rejects_traversal() {
        let result = check_path(&ws(), Path::new("../../windows/system32"));
        assert_eq!(result, Err(PathJailError::DirectoryTraversal));
    }

    #[test]
    fn allows_relative_child() {
        // Non-canonicalized path — should pass syntactic check.
        let result = check_path(&ws(), Path::new("src/main.rs"));
        // Will be Ok or WorkspaceInvalid (since the test dir may not exist on disk).
        assert!(result.is_ok() || result == Err(PathJailError::WorkspaceInvalid));
    }
}
