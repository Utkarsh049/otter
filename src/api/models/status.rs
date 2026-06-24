use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusCode {
    pub id: u8,
    pub description: String,
}

impl StatusCode {
    pub fn queued()                -> Self { Self { id: 1, description: "Queued".to_string() } }
    pub fn processing()            -> Self { Self { id: 2, description: "Processing".to_string() } }
    pub fn accepted()              -> Self { Self { id: 3, description: "Accepted".to_string() } }
    pub fn time_limit_exceeded()   -> Self { Self { id: 4, description: "Time Limit Exceeded".to_string() } }
    pub fn memory_limit_exceeded() -> Self { Self { id: 5, description: "Memory Limit Exceeded".to_string() } }
    pub fn compilation_error()     -> Self { Self { id: 6, description: "Compilation Error".to_string() } }
    pub fn runtime_error()         -> Self { Self { id: 7, description: "Runtime Error".to_string() } }
    pub fn internal_error()        -> Self { Self { id: 8, description: "Internal Error".to_string() } }
}