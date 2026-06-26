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
use crate::execution::languages::{Language, JobContext, CompileResult};
use crate::execution::result::ExecutionResult;
use crate::execution::engine::Engine;
use anyhow::Result;

pub struct Go;

#[async_trait]
impl Language for Go {
    fn id(&self) -> &'static str {
        "go"
    }

    fn name(&self) -> &'static str {
        "Go"
    }

    fn version(&self) -> &'static str {
        "Go 1.20+"
    }

    fn file_extension(&self) -> &'static str {
        "go"
    }

    fn needs_compilation(&self) -> bool {
        true
    }

    async fn compile(&self, ctx: &JobContext) -> Result<CompileResult> {
        // Go compiling command: go build -o program main.go
        let compile_output = Engine::run_command_raw(
            "go",
            &["build", "-o", "program", "main.go"],
            ctx
        ).await?;
        
        Ok(CompileResult {
            success: compile_output.status.is_success(),
            output: compile_output.stderr, // Compiler warnings/errors go to stderr
        })
    }

    async fn run(&self, ctx: &JobContext) -> Result<ExecutionResult> {
        // Run the compiled binary
        Engine::run_command(self.id(), "./program", &[], ctx).await
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
           let mut registry = Self::new();
           registry.register(Arc::new(C));
           registry.register(Arc::new(Cpp));
           registry.register(Arc::new(Python));
           registry.register(Arc::new(JavaScript));
           registry.register(Arc::new(Go)); // Register Go here
           registry
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
