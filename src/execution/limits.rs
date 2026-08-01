#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Limits {
    pub cpu_time_ms: u64,
    pub wall_time_ms: u64,
    pub memory_mb: u64,
    pub max_output_bytes: usize,
    pub max_processes: u32,
    pub disable_sandbox: bool,
    pub slot_id: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu_time_ms: 5000,
            wall_time_ms: 10000,
            memory_mb: 128,
            max_output_bytes: 1_048_576,
            max_processes: 16,
            disable_sandbox: false,
            slot_id: None,
        }
    }
}
