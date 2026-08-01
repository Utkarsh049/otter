pub mod c;
pub mod cpp;
pub mod javascript;
pub mod python;
pub mod registry;

use crate::execution::limits::Limits;
use crate::execution::result::ExecutionResult;
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct JobContext {
    pub id: String,
    pub source_code: String,
    pub stdin: String,
    pub work_dir: std::path::PathBuf,
    pub limits: Limits,
}

#[derive(Debug)]
pub struct CompileOutput {
    pub skipped: bool,
    pub output: String,
    pub success: bool,
}

impl CompileOutput {
    pub fn skipped() -> Self {
        Self {
            skipped: true,
            output: String::new(),
            success: true,
        }
    }
}

#[async_trait]
pub trait Language: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn file_extension(&self) -> &'static str;
    fn needs_compilation(&self) -> bool {
        false
    }
    fn default_limits(&self) -> Limits {
        Limits::default()
    }

    async fn compile(&self, _ctx: &JobContext) -> Result<CompileOutput> {
        Ok(CompileOutput::skipped())
    }

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult>;
}
