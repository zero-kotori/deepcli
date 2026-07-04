#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandGroup {
    Core,
    Support,
    Legacy,
    Experimental,
}

impl CommandGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            CommandGroup::Core => "core",
            CommandGroup::Support => "support",
            CommandGroup::Legacy => "legacy",
            CommandGroup::Experimental => "experimental",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandHelpSummary {
    pub name: &'static str,
    pub listing: &'static str,
    pub summary: &'static str,
    pub usage: &'static [&'static str],
    pub examples: &'static [&'static str],
    pub notes: &'static [&'static str],
    pub running_safe: bool,
    pub group: CommandGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMetadata {
    pub name: &'static str,
    pub running_safe: bool,
    pub group: CommandGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCommandMetadata {
    pub name: &'static str,
    pub successor: &'static str,
    pub policy: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionAliasMetadata {
    pub name: &'static str,
    pub summary: &'static str,
    pub running_safe: bool,
    pub group: CommandGroup,
    pub successor: Option<&'static str>,
    pub policy: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAliasAction {
    SupportBundle,
    CredentialSet,
    CredentialRemove,
    VerifyAccept,
    VerifyGate,
    EnvCompiler,
    EnvInstall,
    SessionCleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandAliasMetadata {
    pub name: &'static str,
    pub canonical: &'static str,
    pub action: CommandAliasAction,
}

const COMMAND_METADATA: &[CommandMetadata] = &[
    command("/help", true, CommandGroup::Support),
    command("/version", false, CommandGroup::Support),
    command("/init", false, CommandGroup::Support),
    command("/status", true, CommandGroup::Core),
    command("/usage", true, CommandGroup::Core),
    command("/quickstart", false, CommandGroup::Support),
    command("/recipes", true, CommandGroup::Support),
    command("/scorecard", true, CommandGroup::Core),
    command("/opportunities", true, CommandGroup::Experimental),
    command("/benchmark", true, CommandGroup::Support),
    command("/round", true, CommandGroup::Core),
    command("/selftest", true, CommandGroup::Support),
    command("/preflight", true, CommandGroup::Core),
    command("/completion", true, CommandGroup::Support),
    command("/diagnose", false, CommandGroup::Support),
    command("/support", false, CommandGroup::Support),
    command("/doctor", false, CommandGroup::Support),
    command("/trace", true, CommandGroup::Core),
    command("/logs", true, CommandGroup::Support),
    command("/privacy", true, CommandGroup::Core),
    command("/context", false, CommandGroup::Support),
    command("/permissions", false, CommandGroup::Core),
    command("/credentials", false, CommandGroup::Core),
    command("/login", false, CommandGroup::Support),
    command("/logout", false, CommandGroup::Support),
    command("/apikey", false, CommandGroup::Legacy),
    command("/config", false, CommandGroup::Core),
    command("/timeout", false, CommandGroup::Support),
    command("/model", false, CommandGroup::Core),
    command("/goal", false, CommandGroup::Core),
    command("/plan", false, CommandGroup::Core),
    command("/fork", true, CommandGroup::Core),
    command("/diff", false, CommandGroup::Core),
    command("/review", false, CommandGroup::Core),
    command("/accept", false, CommandGroup::Core),
    command("/gate", false, CommandGroup::Core),
    command("/verify", false, CommandGroup::Core),
    command("/handoff", false, CommandGroup::Core),
    command("/test", false, CommandGroup::Core),
    command("/compiler", false, CommandGroup::Legacy),
    command("/install", false, CommandGroup::Legacy),
    command("/git", true, CommandGroup::Core),
    command("/web", false, CommandGroup::Support),
    command("/prompt", false, CommandGroup::Support),
    command("/skill", false, CommandGroup::Support),
    command("/agent", false, CommandGroup::Support),
    command("/btw", true, CommandGroup::Core),
    command("/approval", true, CommandGroup::Core),
    command("/session", true, CommandGroup::Core),
    command("/cleanup", true, CommandGroup::Legacy),
    command("/resume", false, CommandGroup::Core),
    command("/rename", false, CommandGroup::Legacy),
    command("/stop", true, CommandGroup::Core),
    command("/quit", true, CommandGroup::Core),
    command("/terminal", true, CommandGroup::Core),
    command("/cmd", false, CommandGroup::Core),
];

const COMPLETION_ALIAS_METADATA: &[CompletionAliasMetadata] = &[
    completion_alias(
        "deepseek",
        "Start deepcli with the DeepSeek provider preset.",
        true,
        CommandGroup::Support,
    ),
    completion_alias(
        "kimi",
        "Start deepcli with the Kimi provider preset.",
        true,
        CommandGroup::Support,
    ),
    completion_alias("ask", "Run a one-shot task.", true, CommandGroup::Support),
    completion_alias(
        "stream",
        "Run a streaming one-shot chat task.",
        true,
        CommandGroup::Support,
    ),
    completion_alias(
        "repl",
        "Compatibility alias for native terminal chat.",
        true,
        CommandGroup::Support,
    ),
    completion_alias(
        "sessions",
        "Alias for session list.",
        true,
        CommandGroup::Support,
    ),
    completion_alias(
        "completions",
        "Alias for completion.",
        true,
        CommandGroup::Support,
    ),
];

const COMMAND_ALIAS_METADATA: &[CommandAliasMetadata] = &[
    command_alias("/support", "/diagnose", CommandAliasAction::SupportBundle),
    command_alias("/login", "/credentials", CommandAliasAction::CredentialSet),
    command_alias("/apikey", "/credentials", CommandAliasAction::CredentialSet),
    command_alias(
        "/logout",
        "/credentials",
        CommandAliasAction::CredentialRemove,
    ),
    command_alias("/accept", "/verify", CommandAliasAction::VerifyAccept),
    command_alias("/gate", "/verify", CommandAliasAction::VerifyGate),
    command_alias("/compiler", "/env", CommandAliasAction::EnvCompiler),
    command_alias("/install", "/env", CommandAliasAction::EnvInstall),
    command_alias("/cleanup", "/session", CommandAliasAction::SessionCleanup),
];

const LEGACY_COMMAND_METADATA: &[LegacyCommandMetadata] = &[
    legacy_command(
        "/apikey",
        "/credentials set",
        "keep as credential setup compatibility alias",
    ),
    legacy_command(
        "/compiler",
        "/doctor compiler",
        "keep as target-first environment compatibility entry",
    ),
    legacy_command(
        "/install",
        "/doctor compiler",
        "keep as local environment setup compatibility entry",
    ),
    legacy_command(
        "/cleanup",
        "/session prune-empty",
        "keep as cleanup compatibility alias",
    ),
    legacy_command(
        "/rename",
        "/session rename --current",
        "keep as active-session rename compatibility alias",
    ),
];

const fn command(name: &'static str, running_safe: bool, group: CommandGroup) -> CommandMetadata {
    CommandMetadata {
        name,
        running_safe,
        group,
    }
}

const fn completion_alias(
    name: &'static str,
    summary: &'static str,
    running_safe: bool,
    group: CommandGroup,
) -> CompletionAliasMetadata {
    CompletionAliasMetadata {
        name,
        summary,
        running_safe,
        group,
        successor: None,
        policy: None,
    }
}

const fn command_alias(
    name: &'static str,
    canonical: &'static str,
    action: CommandAliasAction,
) -> CommandAliasMetadata {
    CommandAliasMetadata {
        name,
        canonical,
        action,
    }
}

const fn legacy_command(
    name: &'static str,
    successor: &'static str,
    policy: &'static str,
) -> LegacyCommandMetadata {
    LegacyCommandMetadata {
        name,
        successor,
        policy,
    }
}

pub(super) fn command_metadata() -> &'static [CommandMetadata] {
    COMMAND_METADATA
}

pub(super) fn legacy_command_metadata() -> &'static [LegacyCommandMetadata] {
    LEGACY_COMMAND_METADATA
}

pub(super) fn command_alias_metadata() -> &'static [CommandAliasMetadata] {
    COMMAND_ALIAS_METADATA
}

pub(super) fn completion_alias_metadata() -> &'static [CompletionAliasMetadata] {
    COMPLETION_ALIAS_METADATA
}

pub(super) fn command_alias_metadata_for(name: &str) -> Option<&'static CommandAliasMetadata> {
    COMMAND_ALIAS_METADATA
        .iter()
        .find(|metadata| metadata.name == name)
}

pub(super) fn command_metadata_for(name: &str) -> Option<&'static CommandMetadata> {
    COMMAND_METADATA
        .iter()
        .find(|metadata| metadata.name == name)
}
