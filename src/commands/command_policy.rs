use serde_json::{json, Value};

use super::{CommandGroup, CommandRouter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandGroupPolicy {
    pub(crate) group: CommandGroup,
    pub(crate) label: &'static str,
    pub(crate) visibility: &'static str,
    pub(crate) policy: &'static str,
}

const COMMAND_GROUP_POLICIES: &[CommandGroupPolicy] = &[
    CommandGroupPolicy {
        group: CommandGroup::Core,
        label: "Core",
        visibility: "primary product and interactive workflow surface",
        policy: "keep stable and prefer for new implementation work",
    },
    CommandGroupPolicy {
        group: CommandGroup::Support,
        label: "Support",
        visibility: "support, setup, diagnostics, and integration surface",
        policy: "keep stable but do not promote as the main task workflow",
    },
    CommandGroupPolicy {
        group: CommandGroup::Legacy,
        label: "Legacy",
        visibility: "compatibility surface only",
        policy: "keep compatibility aliases thin and point users to the documented successor",
    },
    CommandGroupPolicy {
        group: CommandGroup::Experimental,
        label: "Experimental",
        visibility: "opt-in product exploration surface",
        policy: "may change while it remains outside the core workflow",
    },
];

pub(crate) fn command_policy_group_policies() -> &'static [CommandGroupPolicy] {
    COMMAND_GROUP_POLICIES
}

pub(crate) fn command_group_policy_json() -> Vec<Value> {
    command_policy_group_policies()
        .iter()
        .map(|policy| {
            json!({
                "id": policy.group.as_str(),
                "label": policy.label,
                "visibility": policy.visibility,
                "policy": policy.policy,
            })
        })
        .collect()
}

pub(crate) fn legacy_command_policy_json() -> Vec<Value> {
    let mut legacy = CommandRouter::legacy_command_metadata()
        .iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "successor": entry.successor,
                "policy": entry.policy,
                "surface": "slash",
            })
        })
        .collect::<Vec<_>>();

    legacy.extend(
        CommandRouter::completion_alias_metadata()
            .iter()
            .filter(|entry| entry.group == CommandGroup::Legacy)
            .map(|entry| {
                json!({
                    "name": entry.name,
                    "successor": entry.successor.unwrap_or(""),
                    "policy": entry.policy.unwrap_or(""),
                    "surface": "completionAlias",
                })
            }),
    );
    legacy
}
