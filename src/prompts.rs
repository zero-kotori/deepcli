use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prompt {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct PromptStore {
    root: PathBuf,
}

impl PromptStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace.as_ref().join(".deepcli").join("prompts"),
        }
    }

    pub fn builtins() -> BTreeMap<String, Prompt> {
        [
            Prompt {
                name: "code-review".to_string(),
                description: "Review code for bugs, regressions, and missing tests.".to_string(),
                body: "请以代码 review 方式检查当前变更，优先指出 bug、风险和缺失测试。"
                    .to_string(),
            },
            Prompt {
                name: "implementation-plan".to_string(),
                description: "Plan a complex implementation before editing.".to_string(),
                body: "请先说明将阅读哪些文件、修改哪些模块、影响哪些调用链，以及如何验证。"
                    .to_string(),
            },
            Prompt {
                name: "fix-tests".to_string(),
                description: "Analyze a failing test and propose the smallest fix.".to_string(),
                body: "请根据测试失败输出定位原因，并给出最小修复方案。".to_string(),
            },
        ]
        .into_iter()
        .map(|prompt| (prompt.name.clone(), prompt))
        .collect()
    }

    pub fn list(&self) -> Result<Vec<Prompt>> {
        let mut prompts = Self::builtins().into_values().collect::<Vec<_>>();
        if self.root.exists() {
            for entry in fs::read_dir(&self.root)? {
                let entry = entry?;
                if entry.path().extension().and_then(|ext| ext.to_str()) == Some("md") {
                    let name = entry
                        .path()
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or_default()
                        .to_string();
                    prompts.push(Prompt {
                        description: "Custom project prompt".to_string(),
                        body: fs::read_to_string(entry.path())?,
                        name,
                    });
                }
            }
        }
        prompts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(prompts)
    }

    pub fn get(&self, name: &str) -> Result<Prompt> {
        if let Some(prompt) = Self::builtins().remove(name) {
            return Ok(prompt);
        }
        let path = self.root.join(format!("{name}.md"));
        if !path.exists() {
            bail!("prompt `{name}` does not exist");
        }
        Ok(Prompt {
            name: name.to_string(),
            description: "Custom project prompt".to_string(),
            body: fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?,
        })
    }

    pub fn save(&self, name: &str, body: &str) -> Result<PathBuf> {
        validate_prompt_name(name)?;
        fs::create_dir_all(&self.root)?;
        let path = self.root.join(format!("{name}.md"));
        fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }
}

fn validate_prompt_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid prompt name `{name}`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stores_custom_prompt() {
        let dir = tempdir().unwrap();
        let store = PromptStore::new(dir.path());
        store.save("my-prompt", "hello").unwrap();
        assert_eq!(store.get("my-prompt").unwrap().body, "hello");
        assert!(store
            .list()
            .unwrap()
            .iter()
            .any(|prompt| prompt.name == "code-review"));
    }
}
