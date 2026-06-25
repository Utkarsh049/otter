use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::ExecutionResult;
use super::{CompileOutput, JobContext, Language};

pub struct C;

#[async_trait]
impl Language for C {
    fn id(&self)             -> &'static str { "c" }
    fn name(&self)           -> &'static str { "C" }
    fn version(&self)        -> &'static str { "gcc 13" }
    fn file_extension(&self) -> &'static str { "c" }
    fn needs_compilation(&self) -> bool { true }

    async fn compile(&self, ctx: &JobContext) -> Result<CompileOutput> {
        let output = tokio::process::Command::new("gcc")
            .args(&["main.c", "-o", "program", "-O2", "-Wall", "-Wextra"])
            .current_dir(&ctx.work_dir)
            .output()
            .await?;
        
        let success = output.status.success();
        let compiler_output = String::from_utf8_lossy(&output.stderr).to_string()
            + &String::from_utf8_lossy(&output.stdout);
            
        Ok(CompileOutput {
            skipped: false,
            output: compiler_output,
            success,
        })
    }

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult> {
        crate::execution::engine::Engine::run_command("c", "./program", &[], ctx).await
    }
}