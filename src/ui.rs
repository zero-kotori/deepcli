use crate::runtime::AgentRuntime;
use anyhow::Result;

mod native_terminal;
mod resume_picker;

pub use resume_picker::{pick_resume_session, ResumeSelection};

pub async fn run_basic_repl(runtime: AgentRuntime) -> Result<()> {
    native_terminal::run_native_terminal(runtime).await
}
