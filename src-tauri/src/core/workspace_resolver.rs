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

    pub fn resolve_existing_path(workspace: impl AsRef<Path>, requested: &str) -> Result<PathBuf, String> {
        let resolver = Self::new(workspace.as_ref())?;
        resolver.resolve_existing(requested)
    }

    pub fn resolve_create_path(workspace: impl AsRef<Path>, requested: &str) -> Result<PathBuf, String> {
        let resolver = Self::new(workspace.as_ref())?;
        resolver.resolve_for_create(requested)
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

    pub fn resolve_existing(&self, requested: &str) -> Result<PathBuf, String> {
        let normalized = self.normalize_workspace_relative_path(requested)?;
        let target_path = self.canonical_root.join(&normalized);
        
        if !target_path.exists() {
            return Err(format!("PATH_NOT_FOUND: {}", requested));
        }
        
        let canon_target = target_path.canonicalize().map_err(|e| e.to_string())?;
        if !canon_target.starts_with(&self.canonical_root) {
            return Err(format!("PATH_OUTSIDE_WORKSPACE (Symlink Escape): {}", requested));
        }
        
        Ok(target_path)
    }

    pub fn resolve_for_create(&self, requested: &str) -> Result<PathBuf, String> {
        let normalized = self.normalize_workspace_relative_path(requested)?;
        let target_path = self.canonical_root.join(&normalized);
        
        if target_path.exists() {
            return self.resolve_existing(requested);
        }
        
        // Ensure the parent directory is within the workspace
        if let Some(parent) = target_path.parent() {
            if parent.exists() {
                let canon_parent = parent.canonicalize().map_err(|e| e.to_string())?;
                if !canon_parent.starts_with(&self.canonical_root) {
                    return Err(format!("PARENT_OUTSIDE_WORKSPACE (Symlink Escape): {}", requested));
                }
            } else {
                // If parent doesn't exist, we must traverse up until we find an existing parent
                let mut current = parent;
                while let Some(p) = current.parent() {
                    if p.exists() {
                        let canon_p = p.canonicalize().map_err(|e| e.to_string())?;
                        if !canon_p.starts_with(&self.canonical_root) {
                            return Err(format!("ANCESTOR_OUTSIDE_WORKSPACE (Symlink Escape): {}", requested));
                        }
                        break;
                    }
                    current = p;
                }
            }
        }
        
        Ok(target_path)
    }
    
    // Maintain resolve_exact for legacy usage that doesn't care about existence vs creation
    pub fn resolve_exact(&self, requested: &str) -> Result<PathBuf, String> {
        self.resolve_for_create(requested)
    }

    pub fn file_exists(&self, requested: &str) -> Result<bool, String> {
        let target = self.resolve_existing(requested).ok();
        Ok(target.map_or(false, |p| p.is_file()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aura_ws_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("Failed to create test workspace");
        dir
    }

    #[test]
    fn test_normalize_windows_path() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();
        assert_eq!(
            resolver.normalize_workspace_relative_path("src\\main.rs").unwrap(),
            "src/main.rs"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_reject_parent_traversal() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();
        
        let result = resolver.normalize_workspace_relative_path("../outside.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("PATH_OUTSIDE_WORKSPACE"));

        let deep_traversal = resolver.normalize_workspace_relative_path("sub/../../outside.txt");
        assert!(deep_traversal.is_err());
        assert!(deep_traversal.unwrap_err().contains("PATH_OUTSIDE_WORKSPACE"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_reject_absolute_outside_workspace() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();

        // Target an outside directory
        let outside = std::env::temp_dir().join("outside_aura_sentinel_file.txt");
        let result = resolver.normalize_workspace_relative_path(&outside.to_string_lossy());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("PATH_OUTSIDE_WORKSPACE"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_accept_absolute_inside_workspace() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();

        let inside_file = ws.join("src").join("lib.rs");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(&inside_file, "// inside").unwrap();

        let norm = resolver.normalize_workspace_relative_path(&inside_file.to_string_lossy()).unwrap();
        assert_eq!(norm, "src/lib.rs");

        let resolved = resolver.resolve_existing(&inside_file.to_string_lossy()).unwrap();
        assert!(resolved.exists());

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_resolve_create_and_existing() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();

        // Non-existent file cannot be resolved as existing
        let err = resolver.resolve_existing("new_file.txt");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("PATH_NOT_FOUND"));

        // But can be resolved for create
        let create_path = resolver.resolve_for_create("new_file.txt").unwrap();
        std::fs::write(&create_path, "hello world").unwrap();

        // Now it exists and resolves
        let exist_path = resolver.resolve_existing("new_file.txt").unwrap();
        assert_eq!(create_path, exist_path);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_nested_creation_path() {
        let ws = create_test_workspace();
        let resolver = WorkspaceResolver::new(&ws).unwrap();

        let nested = resolver.resolve_for_create("nested/deep/directory/test.rs").unwrap();
        assert!(nested.to_string_lossy().contains("test.rs"));

        // Creating parent dirs and file works within sandbox
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "// ok").unwrap();
        assert!(resolver.resolve_existing("nested/deep/directory/test.rs").is_ok());

        let _ = std::fs::remove_dir_all(&ws);
    }
}
