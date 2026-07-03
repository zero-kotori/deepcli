use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceAuthorization {
    pub workspace: PathBuf,
    pub mode: String,
    pub authorized_at: DateTime<Utc>,
    pub ignore_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSummary {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub agents_files: Vec<FileSummary>,
    pub docs_files: Vec<FileSummary>,
    pub readme_files: Vec<FileSummary>,
    pub git_diff_present: bool,
}

#[derive(Debug, Clone)]
pub struct DeepIgnore {
    root: PathBuf,
    set: GlobSet,
    raw_patterns: Vec<String>,
}

impl DeepIgnore {
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut patterns = default_ignore_patterns();
        let deepignore = root.join(".deepignore");
        if deepignore.exists() {
            let raw = fs::read_to_string(&deepignore)
                .with_context(|| format!("failed to read {}", deepignore.display()))?;
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                patterns.push(line.to_string());
            }
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in &patterns {
            add_glob_pattern(&mut builder, pattern)?;
        }
        let set = builder
            .build()
            .context("failed to build .deepignore matcher")?;

        Ok(Self {
            root,
            set,
            raw_patterns: patterns,
        })
    }

    pub fn is_ignored(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        self.set.is_match(relative)
            || relative.components().any(|component| {
                let value = component.as_os_str().to_string_lossy();
                matches!(
                    value.as_ref(),
                    ".git" | "target" | "node_modules" | ".next" | ".turbo" | ".cache"
                )
            })
    }

    pub fn version_hash(&self) -> String {
        let mut hasher = Sha256::new();
        for pattern in &self.raw_patterns {
            hasher.update(pattern.as_bytes());
            hasher.update(b"\n");
        }
        format!("{:x}", hasher.finalize())
    }
}

pub struct WorkspaceManager {
    root: PathBuf,
    ignore: DeepIgnore,
}

impl WorkspaceManager {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let ignore = DeepIgnore::load(&root)?;
        Ok(Self { root, ignore })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ignore(&self) -> &DeepIgnore {
        &self.ignore
    }

    pub fn authorization_path(&self) -> PathBuf {
        self.root.join(".deepcli").join("authorization.json")
    }

    pub fn load_authorization(&self) -> Result<Option<WorkspaceAuthorization>> {
        let path = self.authorization_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let auth = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(Some(auth))
    }

    pub fn grant_authorization(&self, mode: &str) -> Result<WorkspaceAuthorization> {
        let auth = WorkspaceAuthorization {
            workspace: self.root.clone(),
            mode: mode.to_string(),
            authorized_at: Utc::now(),
            ignore_version: self.ignore.version_hash(),
        };
        let path = self.authorization_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, serde_json::to_vec_pretty(&auth)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(auth)
    }

    pub fn collect_context(&self) -> Result<WorkspaceContext> {
        let mut agents_files = Vec::new();
        let mut docs_files = Vec::new();
        let mut readme_files = Vec::new();

        for entry in self.walk_files(256)? {
            let rel = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path());
            let rel_text = rel.to_string_lossy();
            let name = entry.file_name().to_string_lossy();
            if name == "AGENTS.md" || rel_text.ends_with("/AGENTS.md") {
                agents_files.push(summarize_file(entry.path())?);
            } else if rel_text.starts_with("docs/") || rel_text.starts_with("online-doc/docs/") {
                docs_files.push(summarize_file(entry.path())?);
            } else if name.to_ascii_lowercase().starts_with("readme") {
                readme_files.push(summarize_file(entry.path())?);
            }
        }

        Ok(WorkspaceContext {
            root: self.root.clone(),
            agents_files,
            docs_files,
            readme_files,
            git_diff_present: self.root.join(".git").exists() && has_git_diff(&self.root),
        })
    }

    pub fn walk_files(&self, limit: usize) -> Result<Vec<DirEntry>> {
        self.walk_files_from(&self.root, limit)
    }

    pub fn walk_files_from(&self, base: impl AsRef<Path>, limit: usize) -> Result<Vec<DirEntry>> {
        let base = base.as_ref();
        if !base.starts_with(&self.root) {
            anyhow::bail!("walk base must stay inside workspace: {}", base.display());
        }
        let mut files = Vec::new();
        for entry in WalkDir::new(base)
            .into_iter()
            .filter_entry(|entry| !self.ignore.is_ignored(entry.path()))
        {
            let entry = entry?;
            if entry.file_type().is_file() {
                files.push(entry);
                if files.len() >= limit {
                    break;
                }
            }
        }
        Ok(files)
    }
}

pub fn summarize_file(path: &Path) -> Result<FileSummary> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(FileSummary {
        path: path.to_path_buf(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn has_git_diff(root: &Path) -> bool {
    std::process::Command::new("git")
        .arg("diff")
        .arg("--quiet")
        .current_dir(root)
        .status()
        .map(|status| !status.success())
        .unwrap_or(false)
}

fn add_glob_pattern(builder: &mut GlobSetBuilder, pattern: &str) -> Result<()> {
    let normalized = pattern.trim_start_matches("./");
    if normalized.ends_with('/') {
        builder.add(Glob::new(&format!("{normalized}**"))?);
        builder.add(Glob::new(&format!("**/{normalized}**"))?);
    } else {
        builder.add(Glob::new(normalized)?);
        builder.add(Glob::new(&format!("**/{normalized}"))?);
    }
    Ok(())
}

fn default_ignore_patterns() -> Vec<String> {
    vec![
        ".git/".to_string(),
        ".deepcli/credentials/".to_string(),
        ".deepcli/sessions/".to_string(),
        ".deepcli/logs/".to_string(),
        ".env".to_string(),
        ".env.*".to_string(),
        "*.pem".to_string(),
        "*.key".to_string(),
        "*.p12".to_string(),
        "*.pfx".to_string(),
        "id_rsa".to_string(),
        "id_ed25519".to_string(),
        "credentials.json".to_string(),
        "*-credentials.json".to_string(),
        "node_modules/".to_string(),
        "target/".to_string(),
        "dist/".to_string(),
        "build/".to_string(),
        ".next/".to_string(),
        ".turbo/".to_string(),
        ".cache/".to_string(),
        "coverage/".to_string(),
        "*.log".to_string(),
        "*.sqlite".to_string(),
        "*.db".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ignores_credentials_and_env_files() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        fs::write(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
            "{}",
        )
        .unwrap();
        fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();

        let ignore = DeepIgnore::load(dir.path()).unwrap();
        assert!(ignore.is_ignored(dir.path().join(".env")));
        assert!(ignore.is_ignored(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json")
        ));
        assert!(!ignore.is_ignored(dir.path().join("src.rs")));
    }

    #[test]
    fn context_prefers_docs_and_agents() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("docs/ai")).unwrap();
        fs::write(dir.path().join("AGENTS.md"), "rules").unwrap();
        fs::write(dir.path().join("README.md"), "readme").unwrap();
        fs::write(dir.path().join("docs/ai/REQUIREMENTS.md"), "req").unwrap();
        fs::create_dir_all(dir.path().join("online-doc/docs/lv1")).unwrap();
        fs::write(dir.path().join("online-doc/docs/lv1/README.md"), "lv1").unwrap();

        let manager = WorkspaceManager::new(dir.path()).unwrap();
        let context = manager.collect_context().unwrap();
        assert_eq!(context.agents_files.len(), 1);
        assert_eq!(context.readme_files.len(), 1);
        assert_eq!(context.docs_files.len(), 2);
    }
}
