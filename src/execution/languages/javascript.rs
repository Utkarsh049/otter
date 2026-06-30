use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::ExecutionResult;
use super::{JobContext, Language};

pub struct JavaScript;

#[async_trait]
impl Language for JavaScript {
    fn id(&self)             -> &'static str { "javascript" }
    fn name(&self)           -> &'static str { "JavaScript" }
    fn version(&self)        -> &'static str { "Node 24 LTS" }
    fn file_extension(&self) -> &'static str { "js" }

    fn default_limits(&self) -> crate::execution::limits::Limits {
        crate::execution::limits::Limits {
            max_processes: 32,
            ..crate::execution::limits::Limits::default()
        }
    }

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult> {
        let max_old_space = format!("--max-old-space-size={}", ctx.limits.memory_mb);
        crate::execution::engine::Engine::run_command("javascript", "node", &[&max_old_space, "main.js"], ctx).await
    }
}