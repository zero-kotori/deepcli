use serde_json::{json, Value};

pub(super) fn schema_for(name: &str) -> Value {
    match name {
        "read_file" => object_schema(
            vec![
                ("path", "string"),
                ("start_line", "integer"),
                ("limit", "integer"),
            ],
            &["path"],
        ),
        "list_files" => object_schema(vec![("limit", "integer")], &[]),
        "search" => object_schema(vec![("query", "string"), ("limit", "integer")], &["query"]),
        "write_file" => object_schema(
            vec![
                ("path", "string"),
                ("content", "string"),
                ("approved", "boolean"),
            ],
            &["path", "content"],
        ),
        "apply_patch_or_write" => object_schema(
            vec![
                ("path", "string"),
                ("content", "string"),
                ("patch", "string"),
                ("old", "string"),
                ("new", "string"),
                ("approved", "boolean"),
            ],
            &[],
        ),
        "run_shell" => object_schema(
            vec![
                ("command", "string"),
                ("approved", "boolean"),
                ("writes_files", "boolean"),
                ("requires_network", "boolean"),
                ("timeout_seconds", "integer"),
            ],
            &["command"],
        ),
        "git_diff" => object_schema(vec![("staged", "boolean")], &[]),
        "git_create_branch" => {
            object_schema(vec![("name", "string"), ("approved", "boolean")], &["name"])
        }
        "git_commit_message" => object_schema(Vec::new(), &[]),
        "git_commit" => object_schema(
            vec![("message", "string"), ("approved", "boolean")],
            &["message"],
        ),
        "run_tests" => object_schema(vec![("command", "string")], &[]),
        "check_environment" => object_schema(vec![("target", "string")], &[]),
        "setup_environment" => object_schema(
            vec![
                ("target", "string"),
                ("approved", "boolean"),
                ("install_missing", "boolean"),
                ("smoke_test", "boolean"),
            ],
            &[],
        ),
        "web_search" => object_schema(vec![("query", "string")], &["query"]),
        "prompt_get" => object_schema(vec![("name", "string")], &["name"]),
        "prompt_render" => object_schema(
            vec![
                ("name", "string"),
                ("file", "string"),
                ("variables", "object"),
                ("max_diff_chars", "integer"),
                ("max_file_chars", "integer"),
            ],
            &["name"],
        ),
        "skill_generate" => object_schema(
            vec![
                ("name", "string"),
                ("description", "string"),
                ("approved", "boolean"),
            ],
            &["name", "description"],
        ),
        "skill_run" => object_schema(vec![("name", "string")], &["name"]),
        "spawn_subagent" => object_schema(
            vec![
                ("task", "string"),
                ("depth", "integer"),
                ("write_scope", "array"),
            ],
            &["task"],
        ),
        _ => object_schema(Vec::new(), &[]),
    }
}

fn object_schema(properties: Vec<(&str, &str)>, required: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    for (name, value_type) in properties {
        props.insert(name.to_string(), json!({"type": value_type}));
    }
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false
    })
}
