# Otter — Contributor's Guide: Adding a New Language

Adding a new programming language runtime to the Otter sandbox involves three main steps:
1. Implementing the `Language` trait.
2. Registering the new language struct in `LanguageRegistry`.
3. Adding a Seccomp syscall allowlist inside the Execution Engine.

---

## Step 1: Implement the `Language` Trait
All language runtimes are modeled as structs implementing the `Language` trait defined in `src/execution/languages/mod.rs`.

Create a new file under `src/execution/languages/` (e.g. `src/execution/languages/go.rs`):

```rust
use async_trait::async_trait;
use crate::execution::languages::{Language, JobContext, CompileOutput};
use crate::execution::result::ExecutionResult;
use crate::execution::engine::Engine;
use anyhow::Result;

pub struct Go;

#[async_trait]
impl Language for Go {
    fn id(&self)             -> &'static str { "go" }
    fn name(&self)           -> &'static str { "Go" }
    fn version(&self)        -> &'static str { "Go 1.20+" }
    fn file_extension(&self) -> &'static str { "go" }
    fn needs_compilation(&self) -> bool { true }

    async fn compile(&self, ctx: &JobContext) -> Result<CompileOutput> {
        // Run compilation using tokio::process::Command with a safety timeout
        let output_fut = tokio::process::Command::new("go")
            .args(&["build", "-o", "program", "main.go"])
            .current_dir(&ctx.work_dir)
            .output();
        
        let output = match tokio::time::timeout(std::time::Duration::from_secs(10), output_fut).await {
            Ok(res) => res?,
            Err(_) => {
                return Ok(CompileOutput {
                    skipped: false,
                    output: "Compilation timed out after 10 seconds".to_string(),
                    success: false,
                });
            }
        };
        
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
        // Run the compiled binary inside the sandbox
        Engine::run_command("go", "./program", &[], ctx).await
    }
}
```

---

## Step 2: Register in `LanguageRegistry`
Expose the new module and add the struct instance to the language registry.

1. In `src/execution/languages/mod.rs`, expose the module:
   ```rust
   pub mod go;
   ```
2. In `src/execution/languages/registry.rs`, import your struct and add it to `LanguageRegistry::build()`:
   ```rust
   use super::go::Go;

   impl LanguageRegistry {
       pub fn build() -> Self {
           let mut r = Self { languages: std::collections::HashMap::new() };
           r.register(C);
           r.register(Cpp);
           r.register(Python);
           r.register(JavaScript);
           r.register(Go); // Register Go here
           r
       }
   }
   ```

---

## Step 3: Add Seccomp Allowlist in `engine.rs`
Security-sensitive runtimes require seccomp profiles to prevent malicious system calls.

In `src/execution/engine.rs`, update `syscalls` match arms:
```rust
            let syscalls: &[&str] = match language_id {
                "c" | "cpp" => &[ ... ],
                "python" => &[ ... ],
                "javascript" => &[ ... ],
                "go" => &[
                    "read", "write", "open", "openat", "close", "mmap", "munmap", "mremap",
                    "brk", "rt_sigaction", "rt_sigprocmask", "rt_sigreturn", "sched_yield",
                    "clone", "clone3", "futex", "nanosleep", "exit_group", "getpid",
                    // Add other syscalls required by the Go runtime scheduler...
                ],
                _ => &[]
            };
```

---

## Step 4: Add Unit Tests
Add a hello world test case for the new language in `tests/unit/execution_test.rs` to verify that execution and stdin/stdout work correctly inside the bubblewrap container.
