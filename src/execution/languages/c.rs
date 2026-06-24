use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use super::{CompileOutput, JobContext, Language};

pub struct C;

#[async_trait]
impl Language for C {
    fn id(&self)             -> &'static str { "c" }
    fn name(&self)           -> &'static str { "C" }
    fn version(&self)        -> &'static str { "gcc 13" }
    fn file_extension(&self) -> &'static str { "c" }
    fn needs_compilation(&self) -> bool { true }

    async fn compile(&self, _ctx: &JobContext) -> Result<CompileOutput> {
        // Phase 2: gcc main.c -o program -O2 -Wall -Wextra
        Ok(CompileOutput::skipped())
    }

    async fn run(&self, _ctx: &JobContext) -> Result<ExecutionResult> {
        // Phase 2: ./program
        Ok(ExecutionResult {
            status: ExecutionStatus::Accepted,
            stdout: String::new(), stderr: String::new(),
            compile_output: String::new(),
            time_ms: 0, memory_kb: 0, exit_code: 0,
        })
    }
}