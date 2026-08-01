use crate::execution::languages::{JobContext, Language};
use crate::execution::limits::Limits;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use anyhow::Result;
use nix::sys::resource::{setrlimit, Resource};
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

struct FdGuard(RawFd);

impl Drop for FdGuard {
    fn drop(&mut self) {
        unsafe {
            nix::libc::close(self.0);
        }
    }
}

pub struct Engine;

impl Engine {
    pub async fn execute(
        lang: Arc<dyn Language>,
        source_code: String,
        stdin: String,
        limits: Limits,
    ) -> Result<ExecutionResult> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let work_dir = std::path::PathBuf::from("/tmp").join(format!("otter-{}", job_id));

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
        language_id: &str,
        program: &str,
        args: &[&str],
        ctx: &JobContext,
    ) -> Result<ExecutionResult> {
        let is_javascript = language_id == "javascript";
        let cpu_time_ms = ctx.limits.cpu_time_ms;
        let memory_mb = ctx.limits.memory_mb;
        let max_processes = ctx.limits.max_processes;
        let max_output_bytes = ctx.limits.max_output_bytes;

        let disable_sandbox = ctx.limits.disable_sandbox;
        let mut seccomp_fd: Option<RawFd> = None;
        let mut _seccomp_guard: Option<FdGuard> = None;

        if !disable_sandbox {
            // Compile seccomp filter to a BPF file in the work_dir
            let filter_path = ctx.work_dir.join("filter.bpf");
            {
                let mut filter_file = std::fs::File::create(&filter_path)?;

                let mut filter =
                    libseccomp::ScmpFilterContext::new_filter(libseccomp::ScmpAction::KillProcess)?;
                filter.add_arch(libseccomp::ScmpArch::Native)?;

                let syscalls: &[&str] = match language_id {
                    "c" | "cpp" => &[
                        "read",
                        "write",
                        "open",
                        "openat",
                        "close",
                        "fstat",
                        "stat",
                        "lstat",
                        "lseek",
                        "mmap",
                        "mprotect",
                        "munmap",
                        "mremap",
                        "brk",
                        "rt_sigaction",
                        "rt_sigprocmask",
                        "rt_sigreturn",
                        "ioctl",
                        "access",
                        "select",
                        "poll",
                        "madvise",
                        "getuid",
                        "getgid",
                        "geteuid",
                        "getegid",
                        "exit",
                        "exit_group",
                        "arch_prctl",
                        "futex",
                        "set_tid_address",
                        "set_robust_list",
                        "clock_gettime",
                        "nanosleep",
                        "getcwd",
                        "statx",
                        "newfstatat",
                        "writev",
                        "readv",
                        "execve",
                        "pread64",
                        "pwrite64",
                        "fcntl",
                        "dup",
                        "dup2",
                        "dup3",
                        "getrandom",
                        "sysinfo",
                        "uname",
                        "prlimit64",
                        "getpid",
                        "getppid",
                        "gettid",
                        "sigaltstack",
                        "readlink",
                        "rseq",
                        "prctl",
                        "socket",
                        "connect",
                        "sendto",
                        "recvfrom",
                        "sendmsg",
                        "recvmsg",
                        "setsockopt",
                        "getsockopt",
                        "getsockname",
                        "getpeername",
                        "bind",
                        "listen",
                        "accept",
                        "accept4",
                        "socketpair",
                        "sched_getparam",
                        "sched_setparam",
                        "sched_getscheduler",
                        "sched_setscheduler",
                        "sched_get_priority_max",
                        "sched_get_priority_min",
                        "clock_nanosleep",
                        "faccessat",
                        "faccessat2",
                        "pkey_alloc",
                        "pkey_free",
                        "pkey_mprotect",
                        "readlinkat",
                        "statfs",
                        "signalfd4",
                        "epoll_create1",
                        "epoll_ctl",
                        "epoll_wait",
                        "epoll_pwait",
                        "epoll_pwait2",
                    ],
                    "python" => &[
                        "read",
                        "write",
                        "open",
                        "openat",
                        "close",
                        "fstat",
                        "stat",
                        "lstat",
                        "lseek",
                        "mmap",
                        "mprotect",
                        "munmap",
                        "mremap",
                        "brk",
                        "rt_sigaction",
                        "rt_sigprocmask",
                        "rt_sigreturn",
                        "ioctl",
                        "access",
                        "select",
                        "poll",
                        "madvise",
                        "getuid",
                        "getgid",
                        "geteuid",
                        "getegid",
                        "exit",
                        "exit_group",
                        "arch_prctl",
                        "futex",
                        "set_tid_address",
                        "set_robust_list",
                        "clock_gettime",
                        "nanosleep",
                        "getcwd",
                        "statx",
                        "newfstatat",
                        "writev",
                        "readv",
                        "readlink",
                        "getdents",
                        "getdents64",
                        "fcntl",
                        "dup",
                        "dup2",
                        "dup3",
                        "sysinfo",
                        "getrandom",
                        "uname",
                        "prlimit64",
                        "getpid",
                        "getppid",
                        "gettid",
                        "umask",
                        "sigaltstack",
                        "sched_getaffinity",
                        "sched_yield",
                        "clone",
                        "clone3",
                        "execve",
                        "pread64",
                        "pwrite64",
                        "rseq",
                        "prctl",
                        "socket",
                        "connect",
                        "sendto",
                        "recvfrom",
                        "sendmsg",
                        "recvmsg",
                        "setsockopt",
                        "getsockopt",
                        "getsockname",
                        "getpeername",
                        "bind",
                        "listen",
                        "accept",
                        "accept4",
                        "socketpair",
                        "sched_getparam",
                        "sched_setparam",
                        "sched_getscheduler",
                        "sched_setscheduler",
                        "sched_get_priority_max",
                        "sched_get_priority_min",
                        "clock_nanosleep",
                        "faccessat",
                        "faccessat2",
                        "pkey_alloc",
                        "pkey_free",
                        "pkey_mprotect",
                        "readlinkat",
                        "statfs",
                        "signalfd4",
                        "epoll_create1",
                        "epoll_ctl",
                        "epoll_wait",
                        "epoll_pwait",
                        "epoll_pwait2",
                    ],
                    "javascript" => &[
                        "read",
                        "write",
                        "open",
                        "openat",
                        "close",
                        "fstat",
                        "stat",
                        "lstat",
                        "lseek",
                        "mmap",
                        "mprotect",
                        "munmap",
                        "mremap",
                        "brk",
                        "rt_sigaction",
                        "rt_sigprocmask",
                        "rt_sigreturn",
                        "ioctl",
                        "access",
                        "select",
                        "poll",
                        "madvise",
                        "getuid",
                        "getgid",
                        "geteuid",
                        "getegid",
                        "exit",
                        "exit_group",
                        "arch_prctl",
                        "futex",
                        "set_tid_address",
                        "set_robust_list",
                        "clock_gettime",
                        "nanosleep",
                        "getcwd",
                        "statx",
                        "newfstatat",
                        "writev",
                        "readv",
                        "readlink",
                        "getdents",
                        "getdents64",
                        "fcntl",
                        "dup",
                        "dup2",
                        "dup3",
                        "sysinfo",
                        "getrandom",
                        "uname",
                        "prlimit64",
                        "getpid",
                        "getppid",
                        "gettid",
                        "umask",
                        "sigaltstack",
                        "sched_getaffinity",
                        "sched_yield",
                        "clone",
                        "clone3",
                        "epoll_create1",
                        "epoll_ctl",
                        "epoll_wait",
                        "eventfd2",
                        "timerfd_create",
                        "timerfd_settime",
                        "pipe",
                        "pipe2",
                        "socketpair",
                        "shutdown",
                        "execve",
                        "pread64",
                        "pwrite64",
                        "rseq",
                        "capget",
                        "prctl",
                        "io_uring_setup",
                        "io_uring_enter",
                        "io_uring_register",
                        "epoll_pwait",
                        "epoll_pwait2",
                        "socket",
                        "connect",
                        "sendto",
                        "recvfrom",
                        "sendmsg",
                        "recvmsg",
                        "setsockopt",
                        "getsockopt",
                        "getsockname",
                        "getpeername",
                        "bind",
                        "listen",
                        "accept",
                        "accept4",
                        "sched_getparam",
                        "sched_setparam",
                        "sched_getscheduler",
                        "sched_setscheduler",
                        "sched_get_priority_max",
                        "sched_get_priority_min",
                        "clock_nanosleep",
                        "faccessat",
                        "faccessat2",
                        "pkey_alloc",
                        "pkey_free",
                        "pkey_mprotect",
                        "readlinkat",
                        "statfs",
                        "signalfd4",
                    ],
                    _ => &[],
                };

                for syscall_name in syscalls {
                    if let Ok(syscall) = libseccomp::ScmpSyscall::from_name(syscall_name) {
                        let _ = filter.add_rule(libseccomp::ScmpAction::Allow, syscall);
                    }
                }

                filter.export_bpf(&mut filter_file)?;
            }

            let read_file = std::fs::File::open(&filter_path)?;
            let fd = read_file.into_raw_fd();
            seccomp_fd = Some(fd);
            _seccomp_guard = Some(FdGuard(fd));
        }

        // Resolve the program path
        let resolved = if program.starts_with("/") || program.starts_with("./") {
            std::path::PathBuf::from(program)
        } else if let Ok(path) = std::env::var("PATH") {
            let mut resolved_path = std::path::PathBuf::from(program);
            for dir in std::env::split_paths(&path) {
                let p = dir.join(program);
                if p.exists() {
                    resolved_path = p;
                    break;
                }
            }
            resolved_path
        } else {
            std::path::PathBuf::from(program)
        };

        // Create a pipe to pass child PID from bubblewrap to the parent
        let mut pid_fds = [0; 2];
        let res = unsafe { nix::libc::pipe(pid_fds.as_mut_ptr()) };
        if res == -1 {
            return Err(std::io::Error::last_os_error().into());
        }

        let pid_read_file = unsafe { std::fs::File::from_raw_fd(pid_fds[0]) };
        let mut pid_tokio_file = tokio::fs::File::from_std(pid_read_file);

        let pid_write_raw = pid_fds[1];
        let pid_write_guard = FdGuard(pid_write_raw);

        let mut cmd = if disable_sandbox {
            let mut c = Command::new(&resolved);
            c.args(args);
            c
        } else {
            // Construct bubblewrap arguments
            let mut bwrap_args = vec![
                "--unshare-user".to_string(),
                "--unshare-net".to_string(),
                "--die-with-parent".to_string(),
                "--ro-bind".to_string(),
                "/usr".to_string(),
                "/usr".to_string(),
                "--symlink".to_string(),
                "usr/bin".to_string(),
                "/bin".to_string(),
                "--symlink".to_string(),
                "usr/lib".to_string(),
                "/lib".to_string(),
                "--symlink".to_string(),
                "usr/lib64".to_string(),
                "/lib64".to_string(),
                "--symlink".to_string(),
                "usr/sbin".to_string(),
                "/sbin".to_string(),
                "--proc".to_string(),
                "/proc".to_string(),
                "--dev".to_string(),
                "/dev".to_string(),
                "--tmpfs".to_string(),
                "/tmp".to_string(),
                "--bind".to_string(),
                ctx.work_dir.to_string_lossy().into_owned(),
                "/workspace".to_string(),
                "--chdir".to_string(),
                "/workspace".to_string(),
                "--seccomp".to_string(),
                seccomp_fd.unwrap().to_string(),
                "--info-fd".to_string(),
                pid_write_raw.to_string(),
            ];

            // Bind node runtime's parent directory dynamically if it is in home/nvm
            if resolved.is_absolute() && !resolved.starts_with("/usr") {
                if let Some(parent) = resolved.parent() {
                    bwrap_args.push("--ro-bind".to_string());
                    bwrap_args.push(parent.to_string_lossy().into_owned());
                    bwrap_args.push(parent.to_string_lossy().into_owned());
                }
            }

            bwrap_args.push("--".to_string());
            bwrap_args.push(resolved.to_string_lossy().into_owned());
            for arg in args {
                bwrap_args.push(arg.to_string());
            }

            let mut c = Command::new("bwrap");
            c.args(&bwrap_args);
            c
        };

        cmd.current_dir(&ctx.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);

        let enforce_nproc = !disable_sandbox
            && (std::env::var("APP_ENV").unwrap_or_default() == "production"
                || std::path::Path::new("/.dockerenv").exists()
                || std::env::var("CI").is_ok()
                || std::env::var("OTTER_ENFORCE_NPROC").is_ok()
                || nix::unistd::getuid().is_root());

        let slot_id = ctx.limits.slot_id;

        unsafe {
            cmd.pre_exec(move || {
                if let Some(fd) = seccomp_fd {
                    let res = nix::libc::fcntl(fd, nix::libc::F_SETFD, 0);
                    if res == -1 {
                        return Err(std::io::Error::last_os_error());
                    }

                    let res_pid = nix::libc::fcntl(pid_write_raw, nix::libc::F_SETFD, 0);
                    if res_pid == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else {
                    let my_pid = std::process::id();
                    let msg = format!("{{\"child-pid\": {}}}", my_pid);
                    let _ = nix::libc::write(
                        pid_write_raw,
                        msg.as_ptr() as *const nix::libc::c_void,
                        msg.len(),
                    );
                }

                let cpu_secs = (cpu_time_ms + 999) / 1000;
                let _ = setrlimit(Resource::RLIMIT_CPU, cpu_secs, cpu_secs);

                // Skip RLIMIT_AS for JavaScript because the V8 engine pre-allocates
                // a 4GB virtual memory address space for pointer compression on 64-bit systems.
                // Setting virtual memory limits below 4GB causes Node to crash on startup.
                // Instead, JavaScript physical memory is monitored via VmHWM polling.
                if !is_javascript {
                    let mem_bytes = memory_mb * 1024 * 1024;
                    let _ = setrlimit(Resource::RLIMIT_AS, mem_bytes, mem_bytes);
                }

                if enforce_nproc {
                    let _ = setrlimit(
                        Resource::RLIMIT_NPROC,
                        max_processes as u64,
                        max_processes as u64,
                    );
                }

                let fsize = std::cmp::max(max_output_bytes as u64 * 2, 256 * 1024);
                let _ = setrlimit(Resource::RLIMIT_FSIZE, fsize, fsize);

                let nofile = 128;
                let _ = setrlimit(Resource::RLIMIT_NOFILE, nofile, nofile);

                // Set low CPU scheduling priority (nice = 15) to prevent starving the API server
                let _ = nix::libc::setpriority(nix::libc::PRIO_PROCESS, 0, 15);

                if let Some(slot) = slot_id {
                    if let Ok(Some(num_cores)) =
                        nix::unistd::sysconf(nix::unistd::SysconfVar::_NPROCESSORS_ONLN)
                    {
                        let core_id = slot % (num_cores as usize);
                        let mut cpuset = nix::sched::CpuSet::new();
                        if cpuset.set(core_id).is_ok() {
                            let _ = nix::sched::sched_setaffinity(
                                nix::unistd::Pid::from_raw(0),
                                &cpuset,
                            );
                        }
                    }
                }

                Ok(())
            });
        }

        let mut child = cmd.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get child PID"))?;

        // Close parent's copy of write fd so EOF is sent
        drop(pid_write_guard);

        // Read the actual sandboxed process PID from the pipe
        let mut target_pid = pid;
        {
            let mut pid_buf = vec![0u8; 512];
            match pid_tokio_file.read(&mut pid_buf).await {
                Ok(n) => {
                    tracing::debug!(
                        n = n,
                        content = ?String::from_utf8_lossy(&pid_buf[..n]),
                        "Read sandboxed child PID from info-fd"
                    );
                    if n > 0 {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&pid_buf[..n])
                        {
                            if let Some(child_pid) = val.get("child-pid").and_then(|v| v.as_u64()) {
                                target_pid = child_pid as u32;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(error = ?e, "Failed to read child PID from info-fd");
                }
            }
        }

        // Spawn memory monitor
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let monitor_handle =
            tokio::spawn(
                async move { Self::monitor_memory(target_pid, is_javascript, stop_rx).await },
            );

        // Write stdin to child
        if let Some(mut child_stdin) = child.stdin.take() {
            let stdin_content = ctx.stdin.clone();
            tokio::spawn(async move {
                let _ = child_stdin.write_all(stdin_content.as_bytes()).await;
                let _ = child_stdin.flush().await;
            });
        }

        // Spawn stdout/stderr incremental capped readers
        let mut stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stdout"))?;
        let mut stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stderr"))?;

        let stdout_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut out = Vec::new();
            while out.len() < max_output_bytes {
                let n = match stdout_pipe.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                let to_append = std::cmp::min(n, max_output_bytes - out.len());
                out.extend_from_slice(&buf[..to_append]);
            }
            let mut discard = vec![0u8; 4096];
            while let Ok(n) = stdout_pipe.read(&mut discard).await {
                if n == 0 {
                    break;
                }
            }
            out
        });

        let stderr_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut out = Vec::new();
            while out.len() < max_output_bytes {
                let n = match stderr_pipe.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                let to_append = std::cmp::min(n, max_output_bytes - out.len());
                out.extend_from_slice(&buf[..to_append]);
            }
            let mut discard = vec![0u8; 4096];
            while let Ok(n) = stderr_pipe.read(&mut discard).await {
                if n == 0 {
                    break;
                }
            }
            out
        });

        let start_time = std::time::Instant::now();

        // Wait for child with timeout
        let child_status = tokio::time::timeout(
            std::time::Duration::from_millis(ctx.limits.wall_time_ms),
            child.wait(),
        )
        .await;

        let wall_time_ms = start_time.elapsed().as_millis() as u64;

        // Stop memory monitor
        let _ = stop_tx.send(());
        let peak_memory_kb = monitor_handle.await.unwrap_or(0);

        let (exit_code, status) = match child_status {
            Ok(Ok(status_code)) => {
                if status_code.success() {
                    (0, ExecutionStatus::Accepted)
                } else if let Some(code) = status_code.code() {
                    (code, ExecutionStatus::RuntimeError)
                } else if let Some(signal) = status_code.signal() {
                    (128 + signal, ExecutionStatus::RuntimeError)
                } else {
                    (-1, ExecutionStatus::RuntimeError)
                }
            }
            Ok(Err(e)) => {
                return Err(anyhow::anyhow!("Wait error: {}", e));
            }
            Err(_) => {
                // Timeout exceeded
                // Kill process group
                let pgid = pid as i32;
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(-pgid),
                    nix::sys::signal::Signal::SIGKILL,
                );

                // Reap process
                let _ = child.wait().await;

                (128 + 9, ExecutionStatus::TimeLimitExceeded)
            }
        };

        // Collect outputs
        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();

        let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
        let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();

        // Map limit conditions
        let is_mle = peak_memory_kb >= ctx.limits.memory_mb * 1024
            || exit_code == 134 // V8 abort
            || (status == ExecutionStatus::RuntimeError && peak_memory_kb >= ctx.limits.memory_mb * 1024 * 85 / 100)
            || stderr.contains("MemoryError");

        let is_tle = status == ExecutionStatus::TimeLimitExceeded
            || exit_code == 128 + 24 // SIGXCPU
            || exit_code == 128 + 9; // SIGKILL

        let final_status = if is_mle {
            ExecutionStatus::MemoryLimitExceeded
        } else if is_tle {
            ExecutionStatus::TimeLimitExceeded
        } else {
            status
        };

        Ok(ExecutionResult {
            status: final_status,
            stdout,
            stderr,
            compile_output: String::new(),
            time_ms: wall_time_ms,
            memory_kb: peak_memory_kb,
            exit_code,
        })
    }

    async fn monitor_memory(
        target_pid: u32,
        is_javascript: bool,
        mut stop_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> u64 {
        let status_path = format!("/proc/{}/status", target_pid);
        let mut peak_mem = 0;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1));
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    if let Some(mem) = Self::read_vm_peak(&status_path, is_javascript) {
                        peak_mem = peak_mem.max(mem);
                    }
                    break;
                }
                _ = interval.tick() => {
                    if let Some(mem) = Self::read_vm_peak(&status_path, is_javascript) {
                        peak_mem = peak_mem.max(mem);
                    }
                }
            }
        }
        peak_mem
    }

    fn read_vm_peak(path: &str, is_javascript: bool) -> Option<u64> {
        let file = std::fs::File::open(path).ok()?;
        let reader = std::io::BufReader::new(file);
        let key = if is_javascript { "VmHWM:" } else { "VmPeak:" };
        for line in std::io::BufRead::lines(reader) {
            let line = line.ok()?;
            if line.starts_with(key) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse::<u64>().ok();
                }
            }
        }
        None
    }
}
