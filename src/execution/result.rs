#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    Accepted,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    CompilationError,
    RuntimeError,
    InternalError,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub stdout: String,
    pub stderr: String,
    pub compile_output: String,
    pub time_ms: u64,
    pub memory_kb: u64,
    pub exit_code: i32,
}
