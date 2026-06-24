use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StatusCode {
    pub id: u8,
    pub description: &'static str,
}

impl StatusCode {
    pub fn queued()                -> Self { Self { id: 1, description: "Queued" } }
    pub fn processing()            -> Self { Self { id: 2, description: "Processing" } }
    pub fn accepted()              -> Self { Self { id: 3, description: "Accepted" } }
    pub fn time_limit_exceeded()   -> Self { Self { id: 4, description: "Time Limit Exceeded" } }
    pub fn memory_limit_exceeded() -> Self { Self { id: 5, description: "Memory Limit Exceeded" } }
    pub fn compilation_error()     -> Self { Self { id: 6, description: "Compilation Error" } }
    pub fn runtime_error()         -> Self { Self { id: 7, description: "Runtime Error" } }
    pub fn internal_error()        -> Self { Self { id: 8, description: "Internal Error" } }
}