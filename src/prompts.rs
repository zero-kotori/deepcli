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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptRenderContext {
    pub workspace: String,
    pub cwd: String,
    pub branch: String,
    pub diff: String,
    pub file: String,
    pub file_content: String,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
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
        let mut prompts = Self::builtins();
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
                    prompts.insert(
                        name.clone(),
                        Prompt {
                            description: "Custom project prompt".to_string(),
                            body: fs::read_to_string(entry.path())?,
                            name,
                        },
                    );
                }
            }
        }
        let mut prompts = prompts.into_values().collect::<Vec<_>>();
        prompts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(prompts)
    }

    pub fn get(&self, name: &str) -> Result<Prompt> {
        validate_prompt_name(name)?;
        let path = self.root.join(format!("{name}.md"));
        if path.exists() {
            return Ok(Prompt {
                name: name.to_string(),
                description: "Custom project prompt".to_string(),
                body: fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?,
            });
        }
        if let Some(prompt) = Self::builtins().remove(name) {
            return Ok(prompt);
        }
        bail!("prompt `{name}` does not exist");
    }

    pub fn save(&self, name: &str, body: &str) -> Result<PathBuf> {
        validate_prompt_name(name)?;
        fs::create_dir_all(&self.root)?;
        let path = self.root.join(format!("{name}.md"));
        fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        validate_prompt_name(name)?;
        let path = self.root.join(format!("{name}.md"));
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to delete {}", path.display()))?;
            return Ok(path);
        }
        if Self::builtins().contains_key(name) {
            bail!("cannot delete built-in prompt `{name}`");
        }
        bail!("prompt `{name}` does not exist");
    }
}

pub fn render_prompt_body(body: &str, context: &PromptRenderContext) -> String {
    let mut variables = BTreeMap::from([
        ("workspace".to_string(), context.workspace.clone()),
        ("cwd".to_string(), context.cwd.clone()),
        ("branch".to_string(), context.branch.clone()),
        ("current_branch".to_string(), context.branch.clone()),
        ("diff".to_string(), context.diff.clone()),
        ("git_diff".to_string(), context.diff.clone()),
        ("file".to_string(), context.file.clone()),
        ("current_file".to_string(), context.file.clone()),
        ("file_content".to_string(), context.file_content.clone()),
    ]);
    variables.extend(context.variables.clone());

    let mut rendered = body.to_string();
    for (key, value) in variables {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), &value);
    }
    rendered
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

    #[test]
    fn custom_prompt_overrides_builtin_until_deleted() {
        let dir = tempdir().unwrap();
        let store = PromptStore::new(dir.path());

        store.save("code-review", "custom review").unwrap();
        assert_eq!(store.get("code-review").unwrap().body, "custom review");
        assert_eq!(
            store
                .list()
                .unwrap()
                .iter()
                .filter(|prompt| prompt.name == "code-review")
                .count(),
            1
        );

        store.delete("code-review").unwrap();
        assert!(store
            .get("code-review")
            .unwrap()
            .body
            .contains("代码 review"));
    }

    #[test]
    fn built_in_prompt_cannot_be_deleted() {
        let dir = tempdir().unwrap();
        let store = PromptStore::new(dir.path());

        let error = store.delete("code-review").unwrap_err();

        assert!(error.to_string().contains("cannot delete built-in prompt"));
    }

    #[test]
    fn render_prompt_body_replaces_builtin_aliases_and_extra_variables() {
        let context = PromptRenderContext {
            workspace: "/workspace".to_string(),
            cwd: "/workspace".to_string(),
            branch: "feature/prompt".to_string(),
            diff: "+changed".to_string(),
            file: "src/lib.rs".to_string(),
            file_content: "pub fn ok() {}".to_string(),
            variables: BTreeMap::from([("task".to_string(), "review".to_string())]),
        };

        let rendered = render_prompt_body(
            "{{task}} {{workspace}} {{current_branch}} {{git_diff}} {{current_file}} {{file_content}}",
            &context,
        );

        assert_eq!(
            rendered,
            "review /workspace feature/prompt +changed src/lib.rs pub fn ok() {}"
        );
    }
}
