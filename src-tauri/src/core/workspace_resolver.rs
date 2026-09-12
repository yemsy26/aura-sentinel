use std::path::{Path, PathBuf};

pub struct WorkspaceResolver {
    canonical_root: PathBuf,
}

impl WorkspaceResolver {
    /// Constructs a new WorkspaceResolver, validating that the workspace exists and is a directory.
    pub fn new(workspace: &Path) -> Result<Self, String> {
        if !workspace.exists() {
            return Err(format!("WORKSPACE_NOT_FOUND: {}", workspace.display()));
        }
        if !workspace.is_dir() {
            return Err(format!("WORKSPACE_NOT_DIRECTORY: {}", workspace.display()));
        }
        
        let canonical_root = workspace.canonicalize()
            .map_err(|e| format!("WORKSPACE_CANONICALIZE_FAILED: {}: {}", workspace.display(), e))?;
            
        Ok(Self { canonical_root })
    }

    /// Normalizes a requested path string relative to the workspace, ensuring consistent forward slashes
    /// and preventing path traversal outside the workspace.
    pub fn normalize_workspace_relative_path(
        &self,
        input: &str,
    ) -> Result<String, String> {
        let input_path = Path::new(input);
        
        // If the input is absolute, check if it's inside the workspace
        let mut relative_path = PathBuf::from(input);
        if input_path.is_absolute() {
            // Try to canonicalize input if it exists, otherwise use raw
            let canon_input = input_path.canonicalize()
                .unwrap_or_else(|_| input_path.to_path_buf());
                
            if canon_input.starts_with(&self.canonical_root) {
                relative_path = canon_input.strip_prefix(&self.canonical_root).unwrap().to_path_buf();
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

    /// Resolves a requested file against the exact canonical workspace path.
    pub fn resolve_exact(&self, requested: &str) -> Result<PathBuf, String> {
        let normalized = self.normalize_workspace_relative_path(requested)?;
        let target_path = self.canonical_root.join(&normalized);
        
        // P0 Fix: double check if the final target actually resides in the canonical root (prevent Symlink escapes)
        if target_path.exists() {
            let canon_target = target_path.canonicalize().map_err(|e| e.to_string())?;
            if !canon_target.starts_with(&self.canonical_root) {
                return Err(format!("PATH_OUTSIDE_WORKSPACE (Symlink Escape): {}", requested));
            }
        }
        
        Ok(target_path)
    }

    pub fn file_exists(&self, requested: &str) -> Result<bool, String> {
        let target = self.resolve_exact(requested)?;
        Ok(target.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_windows_path() {
        // Create a real temp dir for canonicalization testing
        let temp_dir = std::env::temp_dir();
        let resolver = WorkspaceResolver::new(&temp_dir).unwrap();
        assert_eq!(
            resolver.normalize_workspace_relative_path("src\\main.rs").unwrap(),
            "src/main.rs"
        );
    }
}
