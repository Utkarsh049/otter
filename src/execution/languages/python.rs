use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::ExecutionResult;
use super::{JobContext, Language};

pub struct Python;

#[async_trait]
impl Language for Python {
    fn id(&self)             -> &'static str { "python" }
    fn name(&self)           -> &'static str { "Python" }
    fn version(&self)        -> &'static str { "3.11" }
    fn file_extension(&self) -> &'static str { "py" }

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult> {
        crate::execution::engine::Engine::run_command("python", "python3", &["main.py"], ctx).await
    }
}