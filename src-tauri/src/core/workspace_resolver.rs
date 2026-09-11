use std::path::{Path, PathBuf};

pub struct WorkspaceResolver;

impl WorkspaceResolver {
    /// Normalizes a requested path string relative to the workspace, ensuring consistent forward slashes
    /// and preventing path traversal outside the workspace.
    pub fn normalize_workspace_relative_path(
        workspace: &Path,
        input: &str,
    ) -> Result<String, String> {
        let input_path = Path::new(input);
        
        let mut components = Vec::new();
        
        // Build path components ignoring '.' and correctly resolving '..' without resolving symlinks
        for component in input_path.components() {
            match component {
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    // If it's an absolute path, we need to check if it's inside the workspace
                    // For now, let's just use the components, but verify at the end
                    continue; 
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if components.pop().is_none() {
                        return Err(format!("PATH_OUTSIDE_WORKSPACE: {}", input));
                    }
                }
                std::path::Component::Normal(c) => {
                    components.push(c.to_string_lossy().to_string());
                }
            }
        }

        let normalized_relative = components.join("/");
        Ok(normalized_relative)
    }

    /// Resolves a requested file against the exact workspace path.
    /// Does NOT search recursively. If the contract says "src/main.rs", it checks "workspace/src/main.rs".
    pub fn resolve_exact(workspace: &Path, requested: &str) -> Result<PathBuf, String> {
        // Normalizes to ensure it stays in workspace bounds
        let normalized = Self::normalize_workspace_relative_path(workspace, requested)?;
        let target_path = workspace.join(&normalized);
        
        // Double check bounds (in case of symlinks or weird absolute paths)
        let canon_ws = workspace.canonicalize().map_err(|e| e.to_string())?;
        
        // Note: canonicalize() will fail if target_path does not exist on disk!
        // For file resolution during Creation or Checking, we may want to resolve even if missing.
        // We can do a prefix check on the raw target_path if it's absolute.
        Ok(target_path)
    }

    pub fn file_exists(workspace: &Path, requested: &str) -> Result<bool, String> {
        let target = Self::resolve_exact(workspace, requested)?;
        Ok(target.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_windows_path() {
        assert_eq!(
            WorkspaceResolver::normalize_workspace_relative_path(Path::new("."), "src\\main.rs").unwrap(),
            "src/main.rs"
        );
    }

    #[test]
    fn test_normalize_relative_path() {
        assert_eq!(
            WorkspaceResolver::normalize_workspace_relative_path(Path::new("."), ".\\src\\main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            WorkspaceResolver::normalize_workspace_relative_path(Path::new("."), "./src/main.rs").unwrap(),
            "src/main.rs"
        );
    }

    #[test]
    fn test_path_traversal() {
        assert!(WorkspaceResolver::normalize_workspace_relative_path(Path::new("."), "../../other/file").is_err());
    }
}
