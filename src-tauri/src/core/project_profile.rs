use std::path::Path;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimaryLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    DotNet,
    HtmlCss,
    Unknown,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProfile {
    pub workspace_path: String,
    pub primary: PrimaryLanguage,
    pub secondary: Vec<PrimaryLanguage>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub has_docker: bool,
    pub has_git: bool,
}

impl ProjectProfile {
    #[allow(dead_code)]
    pub fn detect(workspace: &str) -> Self {
        let ws = Path::new(workspace);
        let mut secondary = Vec::new();
        let mut frameworks = Vec::new();
        let mut package_managers = Vec::new();

        let has_cargo = ws.join("Cargo.toml").exists();
        let has_pyproject = ws.join("pyproject.toml").exists();
        let has_reqs = ws.join("requirements.txt").exists();
        let has_setup_py = ws.join("setup.py").exists();
        let has_pkg_json = ws.join("package.json").exists();
        let has_tsconfig = ws.join("tsconfig.json").exists();
        let has_go_mod = ws.join("go.mod").exists();
        let has_docker = ws.join("Dockerfile").exists() || ws.join("docker-compose.yml").exists();
        let has_git = ws.join(".git").exists();

        // Check package managers
        if has_cargo { package_managers.push("cargo".to_string()); }
        if has_pyproject || has_reqs || has_setup_py {
            if ws.join("poetry.lock").exists() {
                package_managers.push("poetry".to_string());
            } else if ws.join("Pipfile").exists() {
                package_managers.push("pipenv".to_string());
            } else {
                package_managers.push("pip".to_string());
            }
        }
        if has_pkg_json {
            if ws.join("pnpm-lock.yaml").exists() {
                package_managers.push("pnpm".to_string());
            } else if ws.join("yarn.lock").exists() {
                package_managers.push("yarn".to_string());
            } else if ws.join("bun.lockb").exists() {
                package_managers.push("bun".to_string());
            } else {
                package_managers.push("npm".to_string());
            }
        }

        // Framework detection in package.json
        if has_pkg_json {
            if let Ok(content) = std::fs::read_to_string(ws.join("package.json")) {
                if content.contains("\"react\"") { frameworks.push("React".to_string()); }
                if content.contains("\"vue\"") { frameworks.push("Vue".to_string()); }
                if content.contains("\"@angular") { frameworks.push("Angular".to_string()); }
                if content.contains("\"phaser\"") { frameworks.push("Phaser".to_string()); }
                if content.contains("\"tauri\"") || content.contains("\"@tauri-apps") { frameworks.push("Tauri".to_string()); }
                if content.contains("\"express\"") { frameworks.push("Express".to_string()); }
                if content.contains("\"next\"") { frameworks.push("Next.js".to_string()); }
            }
        }

        // Primary & Secondary determination
        let primary = if has_cargo {
            if has_pkg_json {
                secondary.push(if has_tsconfig { PrimaryLanguage::TypeScript } else { PrimaryLanguage::JavaScript });
            }
            PrimaryLanguage::Rust
        } else if has_go_mod {
            PrimaryLanguage::Go
        } else if has_pyproject || has_reqs || has_setup_py {
            if has_pkg_json {
                secondary.push(if has_tsconfig { PrimaryLanguage::TypeScript } else { PrimaryLanguage::JavaScript });
            }
            PrimaryLanguage::Python
        } else if has_tsconfig {
            PrimaryLanguage::TypeScript
        } else if has_pkg_json {
            PrimaryLanguage::JavaScript
        } else if ws.join("index.html").exists() {
            PrimaryLanguage::HtmlCss
        } else {
            PrimaryLanguage::Unknown
        };

        ProjectProfile {
            workspace_path: workspace.to_string(),
            primary,
            secondary,
            frameworks,
            package_managers,
            has_docker,
            has_git,
        }
    }

    #[allow(dead_code)]
    pub fn recommended_test_command(&self) -> Option<String> {
        match self.primary {
            PrimaryLanguage::Rust => Some("cargo test".to_string()),
            PrimaryLanguage::Python => Some("pytest".to_string()),
            PrimaryLanguage::JavaScript | PrimaryLanguage::TypeScript => {
                let pm = self.package_managers.first().map(|s| s.as_str()).unwrap_or("npm");
                Some(format!("{} test", pm))
            },
            PrimaryLanguage::Go => Some("go test ./...".to_string()),
            PrimaryLanguage::DotNet => Some("dotnet test".to_string()),
            PrimaryLanguage::HtmlCss | PrimaryLanguage::Unknown => None,
        }
    }

    #[allow(dead_code)]
    pub fn recommended_dev_command(&self) -> Option<String> {
        if self.frameworks.iter().any(|f| f == "Tauri") {
            return Some("cargo tauri dev".to_string());
        }
        match self.primary {
            PrimaryLanguage::Rust => Some("cargo run".to_string()),
            PrimaryLanguage::Python => Some("python main.py".to_string()),
            PrimaryLanguage::JavaScript | PrimaryLanguage::TypeScript => {
                let pm = self.package_managers.first().map(|s| s.as_str()).unwrap_or("npm");
                Some(format!("{} run dev", pm))
            },
            PrimaryLanguage::Go => Some("go run .".to_string()),
            PrimaryLanguage::DotNet => Some("dotnet run".to_string()),
            PrimaryLanguage::HtmlCss => Some("start index.html".to_string()),
            PrimaryLanguage::Unknown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_empty() {
        let p = ProjectProfile::detect(".");
        assert!(p.workspace_path == ".");
    }
}
