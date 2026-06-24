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

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult> {
        crate::execution::engine::Engine::run_command("node", &["main.js"], ctx).await
    }
}