# Otter

A lightweight, secure code execution engine built in Rust. Run untrusted code safely on cheap hosting — no real VM required.

---

## Why Otter

[Judge0](https://github.com/judge0/judge0) is the go-to open source code runner, but it depends on Linux kernel primitives (`cgroups`, `namespaces`, `ptrace`) that locked container platforms like Heroku, Railway, and Render block at the hypervisor level. It simply does not work on them.

Otter solves this by sandboxing with only **tenant-level** Linux permissions — `rlimit`, `seccomp-bpf` self-applied, and `bubblewrap` with unprivileged user namespaces. No root. No `CAP_SYS_ADMIN`. Works anywhere Docker runs.

```
Judge0:  needs a real VM     ❌ Heroku  ❌ Railway  ❌ Render
Otter:   works in containers ✅ Heroku  ✅ Railway  ✅ Render  ✅ Docker anywhere
```

---

## Features

- **4 languages** — C, C++, Python, JavaScript (Node 24 LTS)
- **Secure sandbox** — rlimit + seccomp-bpf + bubblewrap, 5 independent layers
- **Accurate metrics** — real CPU time, peak memory, exit code per submission
- **Extensible** — adding a new language is one file and one line
- **Production grade** — structured logging, graceful shutdown, bounded concurrency
- **Lightweight** — ~157MB production Docker image (618MB total on disk), ~20MB RAM for the API itself

---

## Quick Start

```bash
git clone https://github.com/your-username/otter
cd otter
docker compose up --build -d
```

API is running at `http://localhost:8080`.

**Submit code:**

```bash
curl -X POST http://localhost:8080/submissions \
  -H "Content-Type: application/json" \
  -d '{
    "language": "python",
    "source_code": "print(\"hello from otter\")",
    "stdin": ""
  }'
```

**Response:**

```json
{
  "token": "550e8400-e29b-41d4-a716-446655440000",
  "status": { "id": 1, "description": "Queued" }
}
```

**Poll for result:**

```bash
curl http://localhost:8080/submissions/550e8400-e29b-41d4-a716-446655440000
```

```json
{
  "token": "550e8400-e29b-41d4-a716-446655440000",
  "status": { "id": 3, "description": "Accepted" },
  "stdout": "hello from otter\n",
  "stderr": null,
  "compile_output": null,
  "time_ms": 42,
  "memory_kb": 8920,
  "exit_code": 0
}
```

---

## Supported Languages

| ID | Name | Version |
|---|---|---|
| `c` | C | gcc 13 |
| `cpp` | C++ | g++ 13 (C++17) |
| `python` | Python | 3.11 |
| `javascript` | JavaScript | Node 24 LTS |

```bash
curl http://localhost:8080/languages
```

---

## API Reference

| Method | Endpoint | Description |
|---|---|---|
| `POST` | `/submissions` | Submit code for execution |
| `GET` | `/submissions/:token` | Poll submission result |
| `POST` | `/submissions/batch` | Submit multiple at once |
| `GET` | `/languages` | List supported languages |
| `GET` | `/health` | Health check |

Full API documentation: [`docs/API.md`](docs/API.md)

### Status Codes

| ID | Description |
|---|---|
| 1 | Queued |
| 2 | Processing |
| 3 | Accepted |
| 4 | Time Limit Exceeded |
| 5 | Memory Limit Exceeded |
| 6 | Compilation Error |
| 7 | Runtime Error |
| 8 | Internal Error |

---

## Configuration

All configuration is via environment variables. Copy `.env.example` to `.env` and adjust.

| Variable | Default | Description |
|---|---|---|
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8080` | Listen port |
| `MAX_CONCURRENT` | `8` | Max simultaneous executions |
| `CPU_LIMIT_MS` | `5000` | CPU time limit per submission |
| `WALL_LIMIT_MS` | `10000` | Wall clock limit per submission |
| `MEMORY_LIMIT_MB` | `128` | Memory limit per submission |
| `MAX_OUTPUT_BYTES` | `1048576` | Max stdout+stderr size (1MB) |
| `RUST_LOG` | `info` | Log level (error/warn/info/debug) |
| `REDIS_URL` | unset | Redis URL for V2 persistence |

---

## Deployment

### Docker (any platform)
```bash
# Build the optimized production image
docker build -f docker/Dockerfile --target runner -t otter:latest .

# Run with privileged flag for secure Bubblewrap sandboxing
docker run -p 8080:8080 --privileged otter:latest
```

### Heroku
```bash
heroku create your-app-name
heroku stack:set container
git push heroku main
```

### Railway / Render
Push your code. Both auto-detect the `Dockerfile` and deploy automatically.

Full deployment guide: [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)

---

## Running Tests

```bash
# Full test suite
./scripts/test_all.sh

# Individual suites
cargo test --lib                    # unit tests
cargo test --test api_test          # integration tests
cargo test --test sandbox_attacks   # security tests
```

---

## Adding a Language

1. Create `src/execution/languages/go.rs` implementing the `Language` trait
2. Add `r.register(Go)` in `src/execution/languages/registry.rs`
3. Add `RUN apt-get install -y golang-go` in `docker/Dockerfile`
4. Add a seccomp profile at `docker/seccomp/go.json`

Full guide: [`docs/ADDING_LANGUAGE.md`](docs/ADDING_LANGUAGE.md)

---

## Security

Otter uses five independent isolation layers per submission:

1. **Concurrency cap** — Tokio semaphore, max N simultaneous jobs
2. **rlimit** — CPU time, memory, file size, process count limits
3. **seccomp-bpf** — per-language syscall allowlist, instant kill on violation
4. **bubblewrap** — network disabled, read-only filesystem, user namespace
5. **Tokio timeout** — wall clock kill switch, entire process tree

See [`docs/SECURITY.md`](docs/SECURITY.md) for threat model and known limitations.

To report a vulnerability: see [`SECURITY.md`](SECURITY.md).

---

## Made for People who don't want to limit themselves 