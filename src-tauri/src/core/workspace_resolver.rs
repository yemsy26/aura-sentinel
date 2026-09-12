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
        
        let canon_ws = workspace.canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());

        // If the input is absolute, check if it's inside the workspace
        let mut relative_path = PathBuf::from(input);
        if input_path.is_absolute() {
            // Try to canonicalize input if it exists, otherwise use raw
            let canon_input = input_path.canonicalize()
                .unwrap_or_else(|_| input_path.to_path_buf());
                
            if canon_input.starts_with(&canon_ws) {
                relative_path = canon_input.strip_prefix(&canon_ws).unwrap().to_path_buf();
            } else {
                return Err(format!("PATH_OUTSIDE_WORKSPACE (Absolute): {}", input));
            }
        }

        let mut components = Vec::new();
        
        // Build path components ignoring '.' and correctly resolving '..'
        for component in relative_path.components() {
            match component {
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    // We already handled absolute paths, so this shouldn't happen, but if it does, ignore
                    continue; 
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if components.pop().is_none() {
                        return Err(format!("PATH_OUTSIDE_WORKSPACE (Relative): {}", input));
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
    pub fn resolve_exact(workspace: &Path, requested: &str) -> Result<PathBuf, String> {
        let normalized = Self::normalize_workspace_relative_path(workspace, requested)?;
        let target_path = workspace.join(&normalized);
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
}
