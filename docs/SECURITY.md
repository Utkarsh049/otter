# Otter — Sandbox Security & Threat Model

This document outlines the security architecture of the Otter code execution sandbox. 

---

## 1. Multi-layered Security Design
Otter achieves host isolation by combining three distinct layers of Linux kernel security:

```
+-------------------------------------------------------------+
|                     Host Linux Kernel                       |
+-------------------------------------------------------------+
                              |
+-------------------------------------------------------------+
|    Layer 1: Bubblewrap (bwrap) Filesystem & Network Jail    |
+-------------------------------------------------------------+
                              |
+-------------------------------------------------------------+
|    Layer 2: Unix Resource Limits (rlimits)                  |
+-------------------------------------------------------------+
                              |
+-------------------------------------------------------------+
|    Layer 3: Seccomp-BPF System Call Allowlist Filters       |
+-------------------------------------------------------------+
                              |
                     [ User Submission ]
```

### Layer 1: Bubblewrap Containment
* **User Namespace (`--unshare-user`)**: Spawns the child process as an unprivileged user inside the sandbox container. Even if the process executes `setuid(0)` or attempts root escapes, it has no root privileges on the host.
* **Network Isolation (`--unshare-net`)**: Fully disables the sandbox network stack. The sandboxed processes cannot initiate outbound connections, download payloads, scan networks, or open listener ports.
* **Filesystem Jail (`--ro-bind`)**: Mounts only necessary system paths (`/usr`, `/bin`, `/lib`, `/lib64`, `/sbin`) as read-only.
  * `/tmp` is mounted as a temporary, memory-backed `tmpfs` unique to the execution instance.
  * `/etc` and other configuration paths are completely excluded. The sandbox cannot read sensitive information like `/etc/passwd`.

### Layer 2: Unix Resource Limits (`rlimits`)
To prevent Denial of Service (DoS) attacks, we apply strict resource limits (`setrlimit`) right before launching the code:
* `RLIMIT_CPU`: Soft and hard limits on maximum CPU time in seconds. If exceeded, the kernel immediately terminates the process with `SIGXCPU`.
* `RLIMIT_AS` / `RLIMIT_DATA`: Restricts virtual memory space allocations. Prevents memory exhaustion attacks (e.g. infinite allocation bombs).
* `RLIMIT_NPROC`: Limits thread and process creation to prevent fork bombs.
* `RLIMIT_FSIZE`: Restricts maximum file size write operations to prevent disk-filling attacks.
* `RLIMIT_NOFILE`: Capped file descriptor handles (128 max) to prevent file descriptor leaks.

### Layer 3: Seccomp Syscall Filtering
We compile and load seccomp-bpf filters dynamically using `libseccomp` before spawning `bwrap`.
* The seccomp filter uses a strict **default-kill** rule (`KillProcess`). Any system call not explicitly present in the allowlist results in the kernel instantly killing the process via `SIGSYS`.
* For C/C++, all thread/process spawning syscalls (`clone`/`clone3`) are excluded, neutralizing fork bomb attempts.
* For interpreted environments (Python/Node.js), thread creation is allowed but network sockets creation (`socket`) and access are restricted.

---

## 2. Threat Model

| Threat Actor | Threat Vector | Target | Containment Strategy |
| :--- | :--- | :--- | :--- |
| **Malicious Submission** | Fork Bomb (`while(1) fork()`) | Host System Exhaustion | Blocked by disabling `clone` in C/C++ seccomp. Container-wide thread cap enforced via `RLIMIT_NPROC`. |
| **Malicious Submission** | Outbound Net Connect (`socket.connect`) | Remote Command Execution / Data Exfiltration | Blocked by `--unshare-net` namespace. Connection attempts fail instantly. |
| **Malicious Submission** | Disk Filler (`write(infinite)`) | Host Disk Exhaustion | Enforced via `RLIMIT_FSIZE`. The kernel terminates the write with `SIGXFSZ` if it exceeds 256KB. |
| **Malicious Submission** | File System Escape (`read("/etc/passwd")`) | Information Disclosure | Blocked because `/etc` is not mounted. System bindings are strictly read-only. |
| **Malicious Submission** | Memory Bomb (`a = []`) | Host Memory Exhaustion | Checked by `RLIMIT_AS` (for VSZ) and a physical memory polling monitor (for Javascript RSS). Triggers `Memory Limit Exceeded` (MLE). |
| **Malicious Submission** | CPU Bomb (`while(1) {}`) | Host CPU Starvation | Terminated via `RLIMIT_CPU` and a parent Tokio timeout monitor sending `SIGKILL` to the process group. |
