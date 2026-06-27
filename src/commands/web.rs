use crate::tools::ToolExecutor;
use anyhow::{bail, Result};
use serde_json::json;

pub(crate) async fn handle_web(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    let query = web_search_query_from_args(&args)?;
    Ok(executor
        .execute("web_search", json!({ "query": query }))
        .await?
        .content)
}

pub(super) fn web_search_query_from_args(args: &[String]) -> Result<String> {
    let query_parts = if args.first().map(String::as_str) == Some("search") {
        &args[1..]
    } else {
        args
    };
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        bail!("/web search requires a query");
    }
    Ok(query)
}
