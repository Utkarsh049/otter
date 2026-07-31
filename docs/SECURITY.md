# Otter — Sandbox Security & Threat Model

This document outlines the security architecture, sandbox design, and threat mitigation strategies implemented in the Otter code execution engine.

---

## 1. Sandbox Concurrency & Architecture Overview

Before code enters the execution sandbox, requests pass through an application-level scheduling layer. The sandbox itself is built using three layers of Linux kernel security.

```text
       ┌──────────────────────────────────────────────────┐
       │               Host Linux Kernel                  │
       └──────────────────────────────────────────────────┘
                                ▲
                                │ [Containment]
       ┌──────────────────────────────────────────────────┐
       │   Layer 1: Bubblewrap Namespace Containment      │
       └──────────────────────────────────────────────────┘
                                ▲
                                │ [Resource Limits]
       ┌──────────────────────────────────────────────────┐
       │   Layer 2: Unix Resource Limits (rlimits)        │
       └──────────────────────────────────────────────────┘
                                ▲
                                │ [Syscall Filter]
       ┌──────────────────────────────────────────────────┐
       │   Layer 3: Seccomp-BPF System Call Allowlist     │
       └──────────────────────────────────────────────────┘
                                ▲
                                │ [Execution]
                       [ User Submission ]
```

### Application-Level Concurrency Control & Rate Limiting
* **Single-Tenant Starvation Defense**: In addition to the global concurrency limit (`MAX_CONCURRENT`), Otter enforces a per-IP fair-share scheduling cap (`MAX_CONCURRENT_PER_IP`). This prevents a single malicious or high-traffic tenant from hogging all execution slots and starving other users.
* **API-Key Rate Limiting**: Requests are rate-limited at the API gateway layer per API key (or client IP fallback) using a sliding-window rate limiter, mitigating automated API denial-of-service (DoS) attempts.

### CPU Core Pinning & Fairness
* **Core Isolation**: To prevent high-CPU submissions from starving other sandboxes, Otter pins each execution slot to a specific CPU core using `sched_setaffinity` round-robin slot allocation. Processes are also run with increased priority niceness, preventing resource starvation on the host machine.

---

## 2. Core Sandbox Security Layers

### Layer 1: Bubblewrap Namespace Containment
Bubblewrap (`bwrap`) is used to isolate the filesystem and network namespaces of the running processes:
* **User Namespace (`--unshare-user`)**: Spawns the child process as an unprivileged user inside the sandbox container. Even if the process executes `setuid(0)` or attempts root escapes, it has no root privileges on the host system.
* **Network Isolation (`--unshare-net`)**: Fully disables the sandbox network stack. The sandboxed processes cannot initiate outbound connections, download payloads, scan networks, or open listener ports.
* **Filesystem Jail (`--ro-bind`)**: Mounts only necessary system paths (`/usr`, `/bin`, `/lib`, `/lib64`, `/sbin`) as read-only.
  * `/tmp` is mounted as a temporary, memory-backed `tmpfs` unique to the execution instance.
  * `/etc` and other sensitive configuration paths are completely excluded, blocking access to host data (e.g. `/etc/passwd`).

### Layer 2: Unix Resource Limits (`rlimits`)
To prevent Denial of Service (DoS) and host exhaustion attacks, we apply strict resource limits (`setrlimit`) to the process tree:
* `RLIMIT_CPU`: Soft and hard limits on maximum CPU time in seconds. If exceeded, the kernel immediately terminates the process with `SIGXCPU`.
* `RLIMIT_AS` / `RLIMIT_DATA`: Restricts virtual memory space allocations. Prevents memory exhaustion attacks (e.g. infinite allocation bombs).
* `RLIMIT_NPROC`: Limits process creation to prevent fork bombs. Enforced via a unique per-run configuration to avoid global starvation.
* `RLIMIT_FSIZE`: Restricts maximum file size write operations to prevent disk-filling attacks.
* `RLIMIT_NOFILE`: Capped file descriptor handles (128 max) to prevent file descriptor leaks.

### Layer 3: Seccomp Syscall Filtering
We compile and load seccomp-bpf filters dynamically using `libseccomp` before spawning `bwrap`.
* **Default-Kill Rule (`KillProcess`)**: The seccomp filter uses a strict default-kill rule. Any system call not explicitly present in the allowlist results in the kernel instantly killing the process via `SIGSYS`.
* **C/C++ Rules**: Thread and process spawning syscalls (`clone`/`clone3`) are excluded, neutralizing fork bomb attempts.
* **Interpreted Rules (Python/Node.js)**: Thread creation is allowed for runtime operation, but network socket creation (`socket`) and access are restricted.

---

## 3. Webhook SSRF Protections
Otter supports outbound webhook callbacks to notify user servers when submissions complete. Because users supply the callback URLs, this presents a Server-Side Request Forgery (SSRF) risk. 
To prevent attacks against internal APIs or local loopback interfaces, Otter implements:
* **Strict DNS Resolution Resolution**: Otter resolves the hostname using `tokio::net::lookup_host` before dispatching the request.
* **IP Blocklist Filtering**: Resolved IP addresses are checked against an exhaustive blocklist covering:
  * IPv4 loopback (`127.0.0.0/8`) and IPv6 loopback (`::1/128`)
  * IPv4/IPv6 private subnets (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `fc00::/7`)
  * Link-local addresses (`169.254.0.0/16`, `fe80::/10`)
  * Unspecified, multicast, and broadcast ranges.
If any resolved IP is blocklisted, the webhook is immediately aborted.

---

## 4. Initialization & Containment Sequence

> [!NOTE]
> **Setup vs. Containment Order**: The layer diagram illustrates the containment hierarchy (the host kernel contains the bubblewrap sandbox, which bounds Unix rlimits, which bounds the seccomp syscall filters).
>
> In terms of initialization sequence, the setup order is the reverse:
> 1. Unix resource limits (Layer 2) and seccomp filters (Layer 3) are configured on the child process.
> 2. The child process then executes `execve` to start Bubblewrap, which finalizes the filesystem/network jail wrap (Layer 1).

---

## 5. Threat Matrix & Mitigation Strategies

| Threat Actor | Threat Vector | Target | Containment Strategy |
| :--- | :--- | :--- | :--- |
| **Malicious Submission** | Fork Bomb (`while(1) fork()`) | Host System Exhaustion | Blocked by disabling `clone` in C/C++ seccomp. Container-wide thread cap enforced via `RLIMIT_NPROC`. |
| **Malicious Submission** | Outbound Net Connect (`socket.connect`) | Remote Command Execution / Data Exfiltration | Blocked by `--unshare-net` namespace. Connection attempts fail instantly. |
| **Malicious Submission** | Disk Filler (`write(infinite)`) | Host Disk Exhaustion | Enforced via `RLIMIT_FSIZE`. The kernel terminates the write with `SIGXFSZ` if it exceeds `MAX_OUTPUT_BYTES` (default 1MB). |
| **Malicious Submission** | File System Escape (`read("/etc/passwd")`) | Information Disclosure | Blocked because `/etc` is not mounted. System bindings are strictly read-only. |
| **Malicious Submission** | Memory Bomb (`a = []`) | Host Memory Exhaustion | Enforced via `RLIMIT_AS` for C/C++/Python virtual memory. For JavaScript, a polling monitor checks peak physical memory RSS (`VmHWM` in `/proc/<pid>/status`) to trigger `Memory Limit Exceeded` (MLE) to prevent Node's V8 engine initialization crashes. |
| **Malicious Submission** | CPU Bomb (`while(1) {}`) | Host CPU Starvation | Terminated via `RLIMIT_CPU` and a parent Tokio timeout monitor sending `SIGKILL` to the process group. |
| **Outbound Webhook** | Server-Side Request Forgery | Internal Private Subnets / Metadata URLs | Webhook resolved IPs are parsed and validated against loopback, private IP ranges, and multicast blocklists before dispatch. |
