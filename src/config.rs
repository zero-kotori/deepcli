use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(rename = "defaultProvider", default = "default_provider")]
    pub default_provider: String,
    #[serde(default = "default_providers")]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub permissions: PermissionConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub usage: UsageConfig,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default = "default_project_name")]
    pub name: String,
    #[serde(default = "default_command_name")]
    pub command: String,
    #[serde(
        rename = "implementationLanguage",
        default = "default_language_runtime"
    )]
    pub implementation_language: String,
    #[serde(default = "default_locale")]
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(rename = "credentialsFile")]
    pub credentials_file: PathBuf,
    #[serde(rename = "acceptanceModel", default)]
    pub acceptance_model: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionConfig {
    #[serde(rename = "defaultMode", default = "default_permission_mode")]
    pub default_mode: String,
    #[serde(rename = "workspaceRead", default = "default_workspace_read")]
    pub workspace_read: String,
    #[serde(rename = "workspaceWrite", default = "default_workspace_write")]
    pub workspace_write: String,
    #[serde(default = "default_shell_policy")]
    pub shell: String,
    #[serde(default = "default_network_policy")]
    pub network: String,
    #[serde(default = "default_git_policy")]
    pub git: String,
    #[serde(rename = "dangerousCommands", default = "default_dangerous_policy")]
    pub dangerous_commands: String,
    #[serde(rename = "approvalPolicy", default = "default_approval_policy")]
    pub approval_policy: String,
    #[serde(
        rename = "dangerousCommandPatterns",
        default = "default_dangerous_patterns"
    )]
    pub dangerous_command_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxConfig {
    #[serde(rename = "enabledByDefault", default = "default_true")]
    pub enabled_by_default: bool,
    #[serde(rename = "workspaceRoot", default = "default_workspace_root")]
    pub workspace_root: PathBuf,
    #[serde(rename = "allowReadWithinWorkspace", default = "default_true")]
    pub allow_read_within_workspace: bool,
    #[serde(rename = "allowNetwork", default = "default_true")]
    pub allow_network: bool,
    #[serde(rename = "allowSystemWrite", default)]
    pub allow_system_write: bool,
    #[serde(rename = "allowDangerousCommands", default)]
    pub allow_dangerous_commands: bool,
    #[serde(rename = "onMissingPermission", default = "default_missing_permission")]
    pub on_missing_permission: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    #[serde(rename = "requirePlanForComplexTasks", default = "default_true")]
    pub require_plan_for_complex_tasks: bool,
    #[serde(rename = "autoReviewer", default = "default_true")]
    pub auto_reviewer: bool,
    #[serde(rename = "maxSubagentDepth", default = "default_subagent_depth")]
    pub max_subagent_depth: u8,
    #[serde(rename = "maxToolIterations", default = "default_tool_iterations")]
    pub max_tool_iterations: usize,
    #[serde(
        rename = "providerTurnTimeoutSeconds",
        default = "default_provider_turn_timeout_seconds"
    )]
    pub provider_turn_timeout_seconds: u64,
    #[serde(default = "default_locale")]
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageConfig {
    #[serde(
        rename = "tokenWarningThreshold",
        default = "default_token_warning_threshold"
    )]
    pub token_warning_threshold: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkConfig {
    #[serde(rename = "webSearch", default = "default_web_search")]
    pub web_search: String,
    #[serde(default)]
    pub proxy: ProxyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProxyConfig {
    pub http: Option<String>,
    pub https: Option<String>,
    #[serde(rename = "noProxy", default)]
    pub no_proxy: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCredentials {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(rename = "apiKey", default)]
    pub api_key: Option<String>,
    #[serde(rename = "apiId", default)]
    pub api_id: Option<String>,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRuntimeConfig {
    pub name: String,
    pub provider_type: String,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub api_id: Option<String>,
    pub capabilities: Vec<String>,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy: Vec<String>,
}

impl AppConfig {
    pub fn load_effective(
        workspace: impl AsRef<Path>,
        explicit_config: Option<&Path>,
    ) -> Result<Self> {
        let workspace = workspace.as_ref();
        let mut config = Self::default();

        if let Some(global) = global_config_path() {
            if global.exists() {
                config = merge_config(config, read_config_file(&global)?);
            }
        }

        let project_config = explicit_config
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.join(".deepcli").join("config.json"));
        if project_config.exists() {
            config = merge_config(config, read_config_file(&project_config)?);
        }

        config.apply_env_overrides();
        Ok(config)
    }

    pub fn provider(&self, name: Option<&str>) -> Result<(&str, &ProviderConfig)> {
        let provider_name = name.unwrap_or(&self.default_provider);
        self.providers
            .get_key_value(provider_name)
            .map(|(key, cfg)| (key.as_str(), cfg))
            .with_context(|| format!("provider `{provider_name}` is not configured"))
    }

    pub fn provider_runtime(
        &self,
        workspace: impl AsRef<Path>,
        name: Option<&str>,
    ) -> Result<ProviderRuntimeConfig> {
        let workspace = workspace.as_ref();
        let (provider_name, provider) = self.provider(name)?;
        let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
        let credentials = if credentials_path.exists() {
            let raw = fs::read_to_string(&credentials_path).with_context(|| {
                format!("failed to read credentials {}", credentials_path.display())
            })?;
            serde_json::from_str::<ProviderCredentials>(&raw).with_context(|| {
                format!("failed to parse credentials {}", credentials_path.display())
            })?
        } else {
            ProviderCredentials {
                provider: Some(provider_name.to_string()),
                name: Some(provider_name.to_string()),
                endpoint: None,
                model: provider.acceptance_model.clone(),
                api_key: env::var(format!(
                    "{}_API_KEY",
                    provider_name.to_ascii_uppercase().replace('-', "_")
                ))
                .ok(),
                api_id: None,
                updated_at: None,
            }
        };

        let env_key = env::var(format!(
            "{}_API_KEY",
            provider_name.to_ascii_uppercase().replace('-', "_")
        ))
        .ok();

        Ok(ProviderRuntimeConfig {
            name: provider_name.to_string(),
            provider_type: provider.provider_type.clone(),
            endpoint: credentials.endpoint,
            model: credentials
                .model
                .or_else(|| provider.acceptance_model.clone()),
            api_key: credentials.api_key.or(env_key),
            api_id: credentials.api_id,
            capabilities: provider.capabilities.clone(),
            http_proxy: self.network.proxy.http.clone(),
            https_proxy: self.network.proxy.https.clone(),
            no_proxy: self.network.proxy.no_proxy.clone(),
        })
    }

    pub fn redacted_provider_runtime(
        &self,
        workspace: impl AsRef<Path>,
        name: Option<&str>,
    ) -> Result<ProviderRuntimeConfig> {
        let mut runtime = self.provider_runtime(workspace, name)?;
        runtime.api_key = runtime.api_key.as_ref().map(|_| "<redacted>".to_string());
        Ok(runtime)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(provider) = env::var("DEEP_CLI_PROVIDER") {
            self.default_provider = provider;
        }
        if let Ok(threshold) = env::var("DEEP_CLI_TOKEN_WARNING_THRESHOLD") {
            if let Ok(value) = threshold.parse::<usize>() {
                self.usage.token_warning_threshold = value;
            }
        }
        if let Ok(timeout) = env::var("DEEP_CLI_PROVIDER_TURN_TIMEOUT_SECONDS") {
            if let Ok(value) = timeout.parse::<u64>() {
                self.agent.provider_turn_timeout_seconds = value;
            }
        }
        if let Ok(iterations) = env::var("DEEP_CLI_MAX_TOOL_ITERATIONS") {
            if let Ok(value) = iterations.parse::<usize>() {
                if value > 0 {
                    self.agent.max_tool_iterations = value;
                }
            }
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            project: ProjectConfig::default(),
            default_provider: default_provider(),
            providers: default_providers(),
            permissions: PermissionConfig::default(),
            sandbox: SandboxConfig::default(),
            agent: AgentConfig::default(),
            usage: UsageConfig::default(),
            network: NetworkConfig::default(),
        }
    }
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            command: default_command_name(),
            implementation_language: default_language_runtime(),
            language: default_locale(),
        }
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            default_mode: default_permission_mode(),
            workspace_read: default_workspace_read(),
            workspace_write: default_workspace_write(),
            shell: default_shell_policy(),
            network: default_network_policy(),
            git: default_git_policy(),
            dangerous_commands: default_dangerous_policy(),
            approval_policy: default_approval_policy(),
            dangerous_command_patterns: default_dangerous_patterns(),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled_by_default: true,
            workspace_root: default_workspace_root(),
            allow_read_within_workspace: true,
            allow_network: true,
            allow_system_write: false,
            allow_dangerous_commands: false,
            on_missing_permission: default_missing_permission(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            require_plan_for_complex_tasks: true,
            auto_reviewer: true,
            max_subagent_depth: default_subagent_depth(),
            max_tool_iterations: default_tool_iterations(),
            provider_turn_timeout_seconds: default_provider_turn_timeout_seconds(),
            language: default_locale(),
        }
    }
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            token_warning_threshold: default_token_warning_threshold(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            web_search: default_web_search(),
            proxy: ProxyConfig::default(),
        }
    }
}

fn read_config_file(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse config {}", path.display()))
}

fn merge_config(_base: AppConfig, overlay: AppConfig) -> AppConfig {
    overlay
}

fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".deepcli").join("config.json"))
}

pub fn absolutize_workspace_path(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

fn default_providers() -> BTreeMap<String, ProviderConfig> {
    let mut providers = BTreeMap::new();
    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            provider_type: "deepseek".to_string(),
            credentials_file: PathBuf::from(".deepcli/credentials/deepseek-credentials.json"),
            acceptance_model: Some("deepseek-v4-pro".to_string()),
            capabilities: vec![
                "streaming".to_string(),
                "reasoner".to_string(),
                "tool_calling".to_string(),
                "json_output".to_string(),
                "context_cache".to_string(),
            ],
        },
    );
    providers.insert(
        "kimi".to_string(),
        ProviderConfig {
            provider_type: "kimi".to_string(),
            credentials_file: PathBuf::from(".deepcli/credentials/kimi-credentials.json"),
            acceptance_model: None,
            capabilities: vec![
                "streaming".to_string(),
                "tool_calling".to_string(),
                "json_output".to_string(),
            ],
        },
    );
    providers
}

fn default_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_project_name() -> String {
    "deep-cli".to_string()
}

fn default_command_name() -> String {
    "deep-cli".to_string()
}

fn default_language_runtime() -> String {
    "rust".to_string()
}

fn default_locale() -> String {
    "zh-CN".to_string()
}

fn default_provider() -> String {
    "deepseek".to_string()
}

fn default_permission_mode() -> String {
    "sandbox".to_string()
}

fn default_workspace_read() -> String {
    "ask_on_first_use".to_string()
}

fn default_workspace_write() -> String {
    "sandbox_then_approval".to_string()
}

fn default_shell_policy() -> String {
    "sandbox_then_approval".to_string()
}

fn default_network_policy() -> String {
    "allow".to_string()
}

fn default_git_policy() -> String {
    "sandbox_then_approval".to_string()
}

fn default_dangerous_policy() -> String {
    "double_confirm".to_string()
}

fn default_approval_policy() -> String {
    "auto_reviewer_then_user".to_string()
}

fn default_dangerous_patterns() -> Vec<String> {
    vec![
        "rm -rf".to_string(),
        "git reset --hard".to_string(),
        "sudo".to_string(),
        "/System/".to_string(),
        "/usr/bin/".to_string(),
        "/usr/local/bin/".to_string(),
    ]
}

fn default_workspace_root() -> PathBuf {
    PathBuf::from(".")
}

fn default_missing_permission() -> String {
    "request_approval".to_string()
}

fn default_subagent_depth() -> u8 {
    2
}

fn default_tool_iterations() -> usize {
    64
}

fn default_provider_turn_timeout_seconds() -> u64 {
    600
}

fn default_token_warning_threshold() -> usize {
    160_000
}

fn default_web_search() -> String {
    "enabled".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn loads_project_config_and_redacts_credentials() {
        let dir = tempdir().unwrap();
        let deepcli = dir.path().join(".deepcli");
        fs::create_dir_all(deepcli.join("credentials")).unwrap();
        fs::write(
            deepcli.join("config.json"),
            r#"{
              "version": 1,
              "defaultProvider": "deepseek",
              "providers": {
                "deepseek": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/deepseek-credentials.json",
                  "acceptanceModel": "configured-model",
                  "capabilities": ["streaming"]
                }
              }
            }"#,
        )
        .unwrap();
        fs::write(
            deepcli.join("credentials/deepseek-credentials.json"),
            r#"{"provider":"deepseek","apiKey":"secret","endpoint":"https://example.test","model":"runtime-model"}"#,
        )
        .unwrap();

        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        let runtime = config.redacted_provider_runtime(dir.path(), None).unwrap();
        assert_eq!(runtime.model.as_deref(), Some("runtime-model"));
        assert_eq!(runtime.api_key.as_deref(), Some("<redacted>"));
    }

    #[test]
    fn provider_runtime_carries_proxy_config() {
        let dir = tempdir().unwrap();
        let deepcli = dir.path().join(".deepcli");
        fs::create_dir_all(&deepcli).unwrap();
        fs::write(
            deepcli.join("config.json"),
            r#"{
              "defaultProvider": "deepseek",
              "providers": {
                "deepseek": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/deepseek-credentials.json"
                }
              },
              "network": {
                "proxy": {
                  "http": "http://127.0.0.1:8080",
                  "https": "http://127.0.0.1:8443",
                  "noProxy": ["localhost"]
                }
              }
            }"#,
        )
        .unwrap();

        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        let runtime = config.provider_runtime(dir.path(), None).unwrap();
        assert_eq!(runtime.http_proxy.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(
            runtime.https_proxy.as_deref(),
            Some("http://127.0.0.1:8443")
        );
        assert_eq!(runtime.no_proxy, vec!["localhost"]);
    }

    #[test]
    fn max_tool_iterations_can_be_overridden_by_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        env::set_var("DEEP_CLI_MAX_TOOL_ITERATIONS", "128");

        let config = AppConfig::load_effective(dir.path(), None).unwrap();

        env::remove_var("DEEP_CLI_MAX_TOOL_ITERATIONS");
        assert_eq!(config.agent.max_tool_iterations, 128);
    }
}
