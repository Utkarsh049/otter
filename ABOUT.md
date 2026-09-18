# About Otter

Otter is a lightweight code-execution engine written in Rust. It accepts source code through an HTTP API, executes the code asynchronously, and returns its output, status, timing, memory usage, and exit code.

Otter is designed for running **untrusted code** on Linux hosts and containers while using relatively lightweight operating-system security features instead of starting a full virtual machine for every submission.

> **Important:** Otter is Linux-focused. It is not completely operating-system independent. Its portability means that it can run across many restricted Linux container and hosting environments without requiring the same host-level features as heavier execution systems.

---

## What Otter Does

A submission contains:

- A language: C, C++, Python, or JavaScript
- Source code
- Optional standard input
- Optional CPU, memory, and wall-clock limits
- Optional webhook callback URL

The API returns a submission token immediately. A worker then executes the job in the background.

The basic flow is:

```text
HTTP request
    |
    v
Validate request and language
    |
    v
Apply queue and concurrency controls
    |
    v
Create a temporary job directory
    |
    v
Write source code to the directory
    |
    v
Compile the source when required
    |
    v
Configure process limits and syscall filtering
    |
    v
Start the program inside Bubblewrap when supported
    |
    v
Capture stdout, stderr, timing, and memory usage
    |
    v
Terminate timed-out processes and clean up files
```

The main execution implementation is in [`src/execution/engine.rs`](src/execution/engine.rs). API request handling is in [`src/api`](src/api), and background queue processing is in [`src/queue`](src/queue).

---

## What Is Sandboxing?

Sandboxing means running a program inside a restricted environment. The program can execute, read its own input, and produce output, but it is prevented from freely accessing the host system.

A sandbox is similar to giving untrusted code a small locked room:

- It has limited CPU and memory.
- It has a restricted filesystem view.
- It cannot normally access the host's private files.
- It cannot normally access the internet or private network services.
- It cannot create unlimited processes.
- It is stopped if it runs for too long.

Sandboxing does not make submitted code trustworthy. It limits the damage if the code is malicious, buggy, or intentionally designed to attack the execution service.

---

## Why Otter Uses Sandboxing

Otter executes source code supplied through an API. Unlike a normal application, where developers control the code being run, Otter must assume that every submission may be hostile.

Without sandboxing, a submission could attempt to:

- Read secrets such as `.env` files, SSH keys, or service credentials.
- Read or modify files belonging to Otter or other users.
- Connect to Redis, databases, internal services, or cloud metadata endpoints.
- Consume all available CPU or memory.
- Create a fork bomb with unlimited child processes.
- Fill the disk with files or output.
- Open excessive files or sockets.
- Keep running forever and exhaust worker capacity.
- Exploit a runtime or operating-system weakness to escape its environment.

Otter therefore combines several controls instead of relying on one mechanism:

```text
Resource limits       -> restrict CPU, memory, processes, files, and output
Seccomp               -> restrict kernel system calls
Bubblewrap            -> restrict users, mounts, filesystem, and networking
Timeouts              -> stop jobs that run for too long
Queue controls        -> prevent unlimited active or waiting jobs
Cleanup               -> remove temporary files and terminate job processes
```

The strongest execution path uses all of these layers. If the host does not support Bubblewrap namespaces, Otter can fall back to raw execution with weaker isolation. That fallback improves portability, but it must be treated as a reduced-security mode rather than as an equivalent sandbox.

---

## Security Model

Submitted code must be treated as potentially malicious. It may attempt to:

- Read sensitive files
- Connect to internal or external networks
- Consume all CPU or memory
- Create an unlimited number of processes
- Fill the disk with output
- Open too many files or sockets
- Run forever
- Escape the execution environment

Otter uses defense in depth: several independent controls are applied so that one control failing does not automatically expose the host.

```text
Application controls
    |
    +-- queue, concurrency, per-IP fairness, per-user fairness, rate limiting

Process controls
    |
    +-- CPU, memory, process, file-size, and file-descriptor limits

Syscall controls
    |
    +-- seccomp-BPF allowlist

Namespace and filesystem controls
    |
    +-- Bubblewrap user, network, and mount isolation

Lifecycle controls
    |
    +-- wall-clock timeout, process-group cleanup, temporary-directory cleanup
```

These controls reduce the impact of malicious submissions, but Otter is not a virtual machine and cannot protect against every kernel, runtime, operating-system, or configuration vulnerability.

---

## Security Layers

### 1. Application-level controls

Before a job is executed, Otter limits how much work the API and worker system will accept.

| Setting | Purpose |
|---|---|
| `MAX_CONCURRENT` | Maximum number of jobs executing at the same time |
| `MAX_QUEUE_DEPTH` | Maximum number of jobs waiting in the queue |
| `MAX_CONCURRENT_PER_IP` | Prevents one client IP from consuming all execution slots |
| `MAX_CONCURRENT_PER_USER` | Prevents one authenticated user/identity from consuming all execution slots |
| `RATE_LIMIT_REQUESTS` | Number of requests allowed in a rate-limit window |
| `RATE_LIMIT_WINDOW_SECONDS` | Length of the rate-limit window |

The global concurrency limit prevents the service from starting unlimited jobs. The per-IP and per-user limits provide fair sharing so that one client or user cannot easily starve other clients.

Rate limiting is optional and is enabled only when both rate-limit variables are configured.

### 2. Temporary job directories

Each job receives a unique directory under `/tmp`, such as:

```text
/tmp/otter-<random-job-id>/
```

The source file, compiled output, and temporary job files are placed there. Otter removes the directory after execution, including when execution returns an error.

### 3. Unix resource limits: `rlimit`

`rlimit` means **resource limit**. Linux allows limits to be applied to individual processes. Otter applies these limits before starting submitted code.

#### CPU time: `RLIMIT_CPU`

`RLIMIT_CPU` limits actual CPU time consumed by the process. It helps stop CPU-heavy programs such as:

```python
while True:
    pass
```

This is different from elapsed time: a process that sleeps may use little CPU but still remain alive, which is why Otter also has a wall-clock timeout.

#### Virtual memory: `RLIMIT_AS`

`RLIMIT_AS` limits the process's virtual address space. It helps prevent memory-allocation attacks.

JavaScript is handled differently because the Node/V8 runtime reserves a large virtual address space during startup. Applying a small virtual-memory limit can make Node fail before user code starts. Otter instead monitors JavaScript's physical memory usage.

#### Process count: `RLIMIT_NPROC`

`RLIMIT_NPROC` restricts how many processes a job can create. This helps defend against fork bombs such as:

```c
while (1) {
    fork();
}
```

Otter enables this enforcement in production, containers, CI, or when `OTTER_ENFORCE_NPROC` is present.

#### File size: `RLIMIT_FSIZE`

`RLIMIT_FSIZE` limits how large a process can make a file. This helps prevent disk-filling attacks and complements the captured-output limit:

```env
MAX_OUTPUT_BYTES=1048576
```

The default output limit is approximately 1 MB.

#### Open files: `RLIMIT_NOFILE`

`RLIMIT_NOFILE` limits open file descriptors. File descriptors represent resources such as files, pipes, and sockets. The limit helps prevent a submission from opening an excessive number of them.

### 4. Seccomp syscall filtering

#### What is a syscall?

A **system call**, or syscall, is a request from a user program to the operating-system kernel. Examples include opening a file, allocating memory, creating a process, or creating a network socket.

#### What is seccomp?

**Seccomp** is short for **secure computing mode**. It allows a process to restrict which syscalls it can use.

Otter builds language-specific seccomp filters with `libseccomp`. The filters use a default-kill policy:

```text
Allowed syscall     -> permitted
Unknown syscall     -> process is killed
```

This is an allowlist design. A program does not receive every syscall simply because it was not explicitly identified as dangerous.

C and C++ have stricter process-creation rules. Interpreted runtimes such as Python and Node.js need additional runtime operations, including thread-related operations, but still run inside the other isolation layers.

#### What is BPF?

BPF means **Berkeley Packet Filter**. In this context, it is a small program evaluated by the kernel when the process attempts a syscall. The seccomp-BPF filter decides whether the syscall is allowed.

Seccomp is not a filesystem sandbox by itself. It is combined with resource limits and Bubblewrap.

### 5. Bubblewrap namespace containment

Bubblewrap, or `bwrap`, is a lightweight Linux sandboxing tool. Otter uses it to create isolated namespaces and a restricted filesystem view.

#### User namespace

With a user namespace, the process receives a separate view of user IDs. A process that appears to be root inside the namespace is not automatically root on the host:

```text
root inside the sandbox != root on the host
```

#### Network namespace

Bubblewrap's network namespace isolates the job's network stack. This prevents submitted code from normally:

- Connecting to the internet
- Scanning internal networks
- Connecting to databases or Redis
- Accessing cloud metadata services
- Opening listener ports for other processes

#### Filesystem and mount namespace

Otter exposes selected runtime directories as read-only, including paths such as `/usr`, `/bin`, `/lib`, and `/sbin`. The job directory is mounted as `/workspace`, and a temporary `/tmp` is created for the job.

The submitted program does not receive the same unrestricted filesystem view as the host. System files are mounted read-only or excluded from the sandbox where appropriate.

For example, code attempting to read the host's sensitive files should not receive the host filesystem view when Bubblewrap is active:

```python
print(open("/etc/passwd").read())
```

### 6. Wall-clock timeout and process cleanup

CPU limits do not stop every type of long-running program. A process may sleep, wait, or keep child processes alive without consuming much CPU.

Otter therefore also uses a wall-clock timeout and runs jobs in a process group. When a job times out, the process group can be terminated instead of only stopping the first process.

This helps clean up jobs that create child processes or otherwise refuse to exit.

### 7. CPU fairness

Otter can assign execution slots to CPU cores using CPU affinity. It also lowers the scheduling priority of submitted processes using a higher niceness value.

These are fairness protections: they reduce the chance that CPU-heavy submissions starve the API server or other jobs. They are not a replacement for CPU time limits.

### 8. Webhook SSRF protection

Otter optionally sends completed results to a user-provided `webhook_url`. This creates an SSRF risk.

**SSRF**, or Server-Side Request Forgery, occurs when an attacker tricks a server into making requests to internal addresses. Examples include:

```text
http://127.0.0.1:6379
http://192.168.1.1
http://169.254.169.254
```

The last address is commonly associated with cloud metadata services.

Before sending a webhook, Otter:

1. Parses the URL.
2. Resolves the hostname.
3. Checks all resolved IP addresses.
4. Blocks loopback, private, link-local, multicast, unspecified, and broadcast addresses.

Loopback webhook access can be enabled for testing with:

```env
ALLOW_LOOPBACK_WEBHOOKS=true
```

When enabled, this allows delivery strictly to loopback addresses (`127.0.0.1`, `::1`) for isolated test harnesses. Other blocked destinations (private subnets, link-local, cloud metadata services, multicast, broadcast) remain strictly forbidden. This should remain `false` for public deployments.

---

## Why Otter Can Run on More Restricted Platforms

The term **platform independent** should be understood carefully.

Otter is not independent of all operating systems. It is primarily Linux-focused because it uses:

- Linux namespaces
- Linux seccomp
- Linux `rlimit`
- Linux process and signal behavior
- Bubblewrap

Its portability is about running across different **Linux hosting environments**, especially environments where the application can run in a container but cannot control the entire host kernel.

### Traditional execution approaches

Many code execution systems depend heavily on one or more of the following:

- Full virtual machines
- Host-level cgroups
- Privileged container operations
- Docker-in-Docker
- `ptrace`
- Host-level namespace configuration

A restricted platform may allow a normal Docker container but block some of these operations.

### Otter's approach

Otter combines application and per-process features:

```text
Application limits       -> queue and concurrency control
rlimit                   -> CPU, memory, process, file, and descriptor limits
seccomp                  -> syscall restrictions
Bubblewrap               -> filesystem, network, and user namespaces
Tokio timeout            -> wall-clock execution limit
```

This can require less infrastructure than launching a virtual machine per submission and may work on platforms where systems requiring cgroups, `ptrace`, or more extensive host control do not work.

The tradeoff is:

```text
Less infrastructure and lower overhead
        versus
A weaker boundary than a full virtual machine
```

### Comparison with a virtual machine

A virtual machine provides a separate guest kernel and is generally a stronger isolation boundary, but it uses more memory, starts more slowly, and requires more infrastructure.

Otter shares the host kernel and therefore depends on the correctness of the host kernel, Bubblewrap, seccomp configuration, and container configuration.

### Comparison with containers

Containers are lightweight and fast, but they also share the host kernel. A container does not automatically provide complete security for arbitrary hostile code.

Otter adds per-submission limits, syscall filtering, namespace isolation, and cleanup on top of its application container.

---

## Sandbox Fallback Mode

At startup, Otter checks whether Bubblewrap and the required namespace features are available.

If they are unavailable, Otter can fall back to raw execution mode. In fallback mode, resource limits are still applied, including:

```text
RLIMIT_CPU
RLIMIT_AS
RLIMIT_FSIZE
RLIMIT_NOFILE
```

> **Note on `RLIMIT_NPROC`:** Process count limits (`RLIMIT_NPROC`) are **not** applied in unjailed raw fallback mode. Without Bubblewrap's unprivileged user namespaces, setting `RLIMIT_NPROC` would constrain the host UID shared by the Otter daemon itself, risking service starvation or crashes. Fork bomb mitigation therefore depends on Bubblewrap.

However, raw mode does not provide the same filesystem, network, and user-namespace isolation as Bubblewrap.

```text
Full sandbox:
resource limits + seccomp + Bubblewrap isolation

Fallback mode:
resource limits, but weaker filesystem/network isolation
```

This fallback allows Otter to remain functional on restricted platforms, but it is a security compromise. Production operators should verify that the full Bubblewrap path is active rather than assuming it is.

To explicitly force raw mode:

```env
DISABLE_SANDBOX=true
```

This should not be enabled for public untrusted workloads unless the reduced security is intentional.

---

## Configuration Overview

The main configuration is loaded from environment variables.

| Variable | Purpose | Typical default |
|---|---|---:|
| `HOST` | HTTP bind address | `0.0.0.0` |
| `PORT` | HTTP port | `8080` |
| `MAX_CONCURRENT` | Maximum simultaneously executing jobs | `8` |
| `CPU_LIMIT_MS` | Default CPU limit per job | `5000` |
| `WALL_LIMIT_MS` | Default wall-clock limit per job | `10000` |
| `MEMORY_LIMIT_MB` | Default memory limit per job | `128` |
| `MAX_OUTPUT_BYTES` | Maximum captured output | `1048576` |
| `MAX_QUEUE_DEPTH` | Maximum queued jobs | `100` |
| `MAX_CONCURRENT_PER_IP` | Per-IP execution limit | `2` |
| `MAX_CONCURRENT_PER_USER` | Per-user execution limit | `2` |
| `DISABLE_SANDBOX` | Force raw execution mode | Automatic detection when unset |
| `REDIS_URL` | Optional Redis queue/store backend | Unset |
| `OTTER_API_KEY` | Optional bearer API key(s) | Unset |
| `OTTER_ADMIN_KEY` | Optional admin bearer key | Unset |
| `RATE_LIMIT_REQUESTS` | Optional request limit | Unset |
| `RATE_LIMIT_WINDOW_SECONDS` | Optional rate-limit window | Unset |
| `ALLOW_LOOPBACK_WEBHOOKS` | Allow loopback webhook targets | `false` |
| `APP_ENV` | Application environment name | `development` |
| `LOG_FORMAT` | Log format; `json` enables JSON logs | `text` |
| `RUST_LOG` | Rust tracing filter | `info` |
| `OTTER_ENFORCE_NPROC` | Force process-count enforcement | Unset |

See [`.env.example`](.env.example) for a safe configuration template.

---

## API Authentication

If neither API key variable is configured and `OTTER_IDENTITY_MODE` is unset, API routes allow anonymous access. If `OTTER_IDENTITY_MODE` is set to `jwt` or `trusted_header`, routes require the corresponding valid identity header (`X-Otter-User-Assertion` or `X-Otter-User-Id`) even when API keys are not configured. Anonymous access is convenient for local development but is not appropriate for an exposed production service.

For production, configure strong secrets such as:

```env
OTTER_API_KEY=<long-random-client-key>
OTTER_ADMIN_KEY=<different-long-random-admin-key>
```

The admin key is used for administrative routes such as `/admin/submissions` and `/admin/metrics`. Secrets should be provided by the deployment platform's secret-management system rather than committed to the repository.

---

## Testing Security Behavior

The following kinds of submissions can be used to verify the protections in a controlled local environment:

### CPU timeout

```python
while True:
    pass
```

Expected result: time-limit failure rather than an indefinitely running job.

### Memory pressure

```python
items = []
while True:
    items.append("x" * 1024 * 1024)
```

Expected result: memory-limit failure or process termination according to the configured limits.

### Network access attempt

```python
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.connect(("1.1.1.1", 80))
```

Expected result: network failure inside the network namespace when Bubblewrap is enabled.

### Forbidden file access

```python
print(open("/etc/shadow").read())
```

Expected result: permission failure, missing file, or sandboxed environment view.

### Fork bomb

```c
#include <unistd.h>
int main() {
    while (1) {
        fork();
    }
    return 0;
}
```

Expected result: job fails, exits, or is terminated by Otter rather than exhausting host processes.

---

## Summary

Otter's design centers on three ideas:

1. **Untrusted code cannot be trusted by policy alone.** It must be constrained by operating-system and process controls.
2. **Defense in depth matters.** Limits, seccomp, namespaces, timeouts, and cleanup work together.
3. **Portability requires intentional compromises.** Otter can run without full virtualization, but fallback mode provides weaker isolation and should only be used when necessary.
