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

pub(super) fn is_running_safe_command_name(name: &str) -> bool {
    matches!(
        name,
        "/help"
            | "/recipes"
            | "/scorecard"
            | "/opportunities"
            | "/benchmark"
            | "/round"
            | "/selftest"
            | "/preflight"
            | "/completion"
            | "/status"
            | "/usage"
            | "/trace"
            | "/logs"
            | "/privacy"
            | "/fork"
            | "/approval"
            | "/session"
            | "/history"
            | "/cleanup"
            | "/btw"
            | "/git"
            | "/stop"
            | "/quit"
            | "/terminal"
    )
}

pub(super) fn command_group_name(name: &str) -> CommandGroup {
    match name {
        "/scorecard" | "/round" | "/preflight" | "/status" | "/usage" | "/trace" | "/privacy"
        | "/permissions" | "/credentials" | "/config" | "/model" | "/goal" | "/plan" | "/fork"
        | "/diff" | "/review" | "/accept" | "/gate" | "/verify" | "/handoff" | "/test" | "/env"
        | "/git" | "/btw" | "/approval" | "/session" | "/resume" | "/stop" | "/quit"
        | "/terminal" => CommandGroup::Core,
        "/about" | "/auth" | "/apikey" | "/key" | "/check" | "/docker" | "/compiler" | "/setup"
        | "/install" | "/history" | "/cleanup" | "/rename" => CommandGroup::Legacy,
        "/opportunities" => CommandGroup::Experimental,
        _ => CommandGroup::Support,
    }
}
