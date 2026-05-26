use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub max_depth: u8,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedSkill {
    pub metadata_path: PathBuf,
    pub instruction_path: PathBuf,
    pub metadata: SkillMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadedSkill {
    pub metadata: SkillMetadata,
    pub instructions: String,
}

#[derive(Debug, Clone)]
pub struct SkillStore {
    root: PathBuf,
}

impl SkillStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace.as_ref().join(".deepcli").join("skills"),
        }
    }

    pub fn discover(&self) -> Result<Vec<SkillMetadata>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut skills: Vec<SkillMetadata> = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let metadata_path = entry.path().join("skill.json");
            if metadata_path.exists() {
                let raw = fs::read_to_string(&metadata_path)?;
                skills.push(serde_json::from_str(&raw)?);
            }
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(skills)
    }

    pub fn generate(&self, name: &str, description: &str) -> Result<GeneratedSkill> {
        validate_skill_name(name)?;
        let skill_dir = self.root.join(name);
        fs::create_dir_all(&skill_dir)
            .with_context(|| format!("failed to create {}", skill_dir.display()))?;
        let metadata = SkillMetadata {
            name: name.to_string(),
            description: description.to_string(),
            trigger: format!("Use when the user asks for {description}."),
            max_depth: 1,
            created_at: Utc::now(),
        };
        let metadata_path = skill_dir.join("skill.json");
        let instruction_path = skill_dir.join("SKILL.md");
        fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)
            .with_context(|| format!("failed to write {}", metadata_path.display()))?;
        fs::write(
            &instruction_path,
            format!(
                "# {name}\n\n## Purpose\n\n{description}\n\n## Trigger\n\n{}\n\n## Workflow\n\n1. Confirm the task matches this skill.\n2. Read only the files needed for the task.\n3. Produce the smallest safe change and verify it.\n\n## Limits\n\n- Max sub-agent depth: 1.\n- All file, shell, network, and git operations still go through deep-cli permissions.\n",
                metadata.trigger
            ),
        )
        .with_context(|| format!("failed to write {}", instruction_path.display()))?;
        Ok(GeneratedSkill {
            metadata_path,
            instruction_path,
            metadata,
        })
    }

    pub fn load(&self, name: &str) -> Result<LoadedSkill> {
        validate_skill_name(name)?;
        let skill_dir = self.root.join(name);
        let metadata_path = skill_dir.join("skill.json");
        let instruction_path = skill_dir.join("SKILL.md");
        let metadata = serde_json::from_str(
            &fs::read_to_string(&metadata_path)
                .with_context(|| format!("failed to read {}", metadata_path.display()))?,
        )?;
        let instructions = fs::read_to_string(&instruction_path)
            .with_context(|| format!("failed to read {}", instruction_path.display()))?;
        Ok(LoadedSkill {
            metadata,
            instructions,
        })
    }
}

fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid skill name `{name}`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generates_and_loads_skill() {
        let dir = tempdir().unwrap();
        let store = SkillStore::new(dir.path());
        store
            .generate("review-helper", "review Rust changes")
            .unwrap();
        let skills = store.discover().unwrap();
        assert_eq!(skills.len(), 1);
        let loaded = store.load("review-helper").unwrap();
        assert!(loaded.instructions.contains("review Rust changes"));
    }
}
