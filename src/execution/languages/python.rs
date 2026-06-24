use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use super::{JobContext, Language, CompileOutput};

pub struct Python;

#[async_trait]
impl Language for Python {
    fn id(&self)             -> &'static str { "python" }
    fn name(&self)           -> &'static str { "Python" }
    fn version(&self)        -> &'static str { "3.11" }
    fn file_extension(&self) -> &'static str { "py" }

    async fn run(&self, _ctx: &JobContext) -> Result<ExecutionResult> {
        // Phase 2: python3 main.py
        Ok(ExecutionResult {
            status: ExecutionStatus::Accepted,
            stdout: String::new(), stderr: String::new(),
            compile_output: String::new(),
            time_ms: 0, memory_kb: 0, exit_code: 0,
        })
    }
}