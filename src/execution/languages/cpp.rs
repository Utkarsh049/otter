use anyhow::Result;
use async_trait::async_trait;
use crate::execution::result::ExecutionResult;
use super::{CompileOutput, JobContext, Language};

pub struct Cpp;

#[async_trait]
impl Language for Cpp {
    fn id(&self)             -> &'static str { "cpp" }
    fn name(&self)           -> &'static str { "C++" }
    fn version(&self)        -> &'static str { "g++ 13" }
    fn file_extension(&self) -> &'static str { "cpp" }
    fn needs_compilation(&self) -> bool { true }

    async fn compile(&self, ctx: &JobContext) -> Result<CompileOutput> {
        let output = tokio::process::Command::new("g++")
            .args(&["main.cpp", "-o", "program", "-O2", "-std=c++17"])
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
        crate::execution::engine::Engine::run_command("./program", &[], ctx).await
    }
}