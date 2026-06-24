use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use super::{JobContext, Language, CompileOutput};

pub struct JavaScript;

#[async_trait]
impl Language for JavaScript {
    fn id(&self)             -> &'static str { "javascript" }
    fn name(&self)           -> &'static str { "JavaScript" }
    fn version(&self)        -> &'static str { "Node 24 LTS" }
    fn file_extension(&self) -> &'static str { "js" }

    async fn run(&self, _ctx: &JobContext) -> Result<ExecutionResult> {
        // Phase 2: node main.js
        Ok(ExecutionResult {
            status: ExecutionStatus::Accepted,
            stdout: String::new(), stderr: String::new(),
            compile_output: String::new(),
            time_ms: 0, memory_kb: 0, exit_code: 0,
        })
    }
}