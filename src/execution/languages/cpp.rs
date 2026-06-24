use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use super::{CompileOutput, JobContext, Language};

pub struct Cpp;

#[async_trait]
impl Language for Cpp {
    fn id(&self)             -> &'static str { "cpp" }
    fn name(&self)           -> &'static str { "C++" }
    fn version(&self)        -> &'static str { "g++ 13" }
    fn file_extension(&self) -> &'static str { "cpp" }
    fn needs_compilation(&self) -> bool { true }

    async fn compile(&self, _ctx: &JobContext) -> Result<CompileOutput> {
        // Phase 2: g++ main.cpp -o program -O2 -std=c++17
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