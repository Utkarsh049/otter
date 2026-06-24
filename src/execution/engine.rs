use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;
use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use crate::execution::languages::{Language, JobContext};
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use crate::execution::limits::Limits;

pub struct Engine;

impl Engine {
    pub async fn execute(
        lang: Arc<dyn Language>,
        source_code: String,
        stdin: String,
        limits: Limits,
    ) -> Result<ExecutionResult> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let work_dir = std::path::PathBuf::from("/dev/shm").join(format!("otter-{}", job_id));
        
        tokio::fs::create_dir_all(&work_dir).await?;
        
        let file_name = format!("main.{}", lang.file_extension());
        let source_path = work_dir.join(&file_name);
        tokio::fs::write(&source_path, &source_code).await?;
        
        let ctx = JobContext {
            id: job_id,
            source_code,
            stdin,
            work_dir: work_dir.clone(),
            limits,
        };
        
        let result = Self::run_job(lang.as_ref(), &ctx).await;
        
        // Always clean up the temp directory
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        
        result
    }

    async fn run_job(lang: &dyn Language, ctx: &JobContext) -> Result<ExecutionResult> {
        if lang.needs_compilation() {
            let compile_res = lang.compile(ctx).await?;
            if !compile_res.success {
                return Ok(ExecutionResult {
                    status: ExecutionStatus::CompilationError,
                    stdout: String::new(),
                    stderr: String::new(),
                    compile_output: compile_res.output,
                    time_ms: 0,
                    memory_kb: 0,
                    exit_code: -1,
                });
            }
        }
        
        lang.run(ctx).await
    }

    pub async fn run_command(
        program: &str,
        args: &[&str],
        ctx: &JobContext,
    ) -> Result<ExecutionResult> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(&ctx.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        
        let mut child = cmd.spawn()?;
        let pid = child.id().ok_or_else(|| anyhow::anyhow!("Failed to get child PID"))?;
        
        // Spawn memory monitor
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let monitor_handle = tokio::spawn(async move {
            Self::monitor_memory(pid, stop_rx).await
        });
        
        // Write stdin to child
        if let Some(mut child_stdin) = child.stdin.take() {
            let stdin_content = ctx.stdin.clone();
            tokio::spawn(async move {
                let _ = child_stdin.write_all(stdin_content.as_bytes()).await;
                let _ = child_stdin.flush().await;
            });
        }
        
        let start_time = std::time::Instant::now();
        
        // Wait for child to exit and collect stdout/stderr
        let output = child.wait_with_output().await?;
        let wall_time_ms = start_time.elapsed().as_millis() as u64;
        
        // Stop memory monitor
        let _ = stop_tx.send(());
        let peak_memory_kb = monitor_handle.await.unwrap_or(0);
        
        // Get exit code and map status
        let (exit_code, status) = if output.status.success() {
            (0, ExecutionStatus::Accepted)
        } else if let Some(code) = output.status.code() {
            (code, ExecutionStatus::RuntimeError)
        } else if let Some(signal) = output.status.signal() {
            let code = 128 + signal;
            (code, ExecutionStatus::RuntimeError)
        } else {
            (-1, ExecutionStatus::RuntimeError)
        };
        
        // Map common signals / codes
        let mapped_status = if exit_code == 139 {
            ExecutionStatus::RuntimeError // TIMELINE says "0=Accepted, non-zero=RuntimeError, 139=segfault" but status is RuntimeError. Oh wait, is segfault still RuntimeError status? Yes, 139 is segfault.
        } else {
            status
        };
        
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        
        Ok(ExecutionResult {
            status: mapped_status,
            stdout,
            stderr,
            compile_output: String::new(),
            time_ms: wall_time_ms,
            memory_kb: peak_memory_kb,
            exit_code,
        })
    }

    async fn monitor_memory(pid: u32, mut stop_rx: tokio::sync::oneshot::Receiver<()>) -> u64 {
        let status_path = format!("/proc/{}/status", pid);
        let mut peak_mem = 0;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1));
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    if let Some(mem) = Self::read_vm_peak(&status_path) {
                        peak_mem = peak_mem.max(mem);
                    }
                    break;
                }
                _ = interval.tick() => {
                    if let Some(mem) = Self::read_vm_peak(&status_path) {
                        peak_mem = peak_mem.max(mem);
                    } else {
                        break;
                    }
                }
            }
        }
        peak_mem
    }

    fn read_vm_peak(path: &str) -> Option<u64> {
        let file = std::fs::File::open(path).ok()?;
        let reader = std::io::BufReader::new(file);
        for line in std::io::BufRead::lines(reader) {
            let line = line.ok()?;
            if line.starts_with("VmPeak:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse::<u64>().ok();
                }
            }
        }
        None
    }
}