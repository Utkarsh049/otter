# Otter — API Documentation

This document describes the API endpoints exposed by the Otter code execution engine.

By default, the server runs on `http://localhost:8080`.

All POST requests must include the header:
```http
Content-Type: application/json
```

---

## 1. Health Check
Checks the server's lifecycle and current version.

* **URL**: `/health`
* **Method**: `GET`
* **Response Status**: `200 OK`
* **Response Body**:
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/health
```

---

## 2. List Languages
Returns all compilers and runtimes configured on the sandbox container.

* **URL**: `/languages`
* **Method**: `GET`
* **Response Status**: `200 OK`
* **Response Body**:
```json
[
  {
    "id": "c",
    "name": "C",
    "version": "GCC 11+"
  },
  {
    "id": "cpp",
    "name": "C++",
    "version": "G++ 11+"
  },
  {
    "id": "python",
    "name": "Python",
    "version": "Python 3.10+"
  },
  {
    "id": "javascript",
    "name": "JavaScript",
    "version": "NodeJS 18+"
  }
]
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/languages
```

---

## 3. Submit Code
Creates a single code execution task. The execution runs asynchronously in the background.

* **URL**: `/submissions`
* **Method**: `POST`
* **Response Status**: `201 Created`
* **Request Fields**:
  * `language` (string, required): One of `c`, `cpp`, `python`, `javascript`.
  * `source_code` (string, required): The complete source code.
  * `stdin` (string, optional): Input bytes to feed into standard input. Defaults to empty.
  * `cpu_time_limit_ms` (integer, optional): CPU execution cap. Defaults to server setting.
  * `memory_limit_mb` (integer, optional): Peak memory boundary. Defaults to server setting.
  * `wall_time_limit_ms` (integer, optional): Total wall-clock execution limit.
  * `webhook_url` (string, optional): HTTP/HTTPS endpoint to receive execution results asynchronously. SSRF protection is enforced.

* **Request Body**:
```json
{
  "language": "python",
  "source_code": "import sys\ndata = sys.stdin.read()\nprint(f'Hello {data}!')",
  "stdin": "Otter",
  "cpu_time_limit_ms": 1000,
  "memory_limit_mb": 64,
  "wall_time_limit_ms": 2000,
  "webhook_url": "https://yourserver.com/callback"
}
```

* **Response Body**:
```json
{
  "token": "79b32e60-84cf-4d92-8086-5386db49f9be",
  "status": {
    "id": 1,
    "description": "Queued"
  },
  "stdout": null,
  "stderr": null,
  "compile_output": null,
  "time_ms": null,
  "memory_kb": null,
  "exit_code": null
}
```

* **Example Request**:
```bash
curl -X POST http://localhost:8080/submissions \
  -H "Content-Type: application/json" \
  -d '{
    "language": "python",
    "source_code": "print(\"hello\")"
  }'
```

---

## 4. List Submissions
Returns a list of all active or recently executed submissions.

* **URL**: `/admin/submissions`
* **Method**: `GET`
* **Response Status**: `200 OK` (requires authorization bearer token if API key is configured)
* **Response Body**:
```json
[
  {
    "token": "79b32e60-84cf-4d92-8086-5386db49f9be",
    "status": {
      "id": 3,
      "description": "Accepted"
    },
    "stdout": "Hello, World!\n",
    "stderr": "",
    "compile_output": "",
    "time_ms": 12,
    "memory_kb": 8012,
    "exit_code": 0
  }
]
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/admin/submissions -H "Authorization: Bearer <your-key>"
```

---

## 5. Get Submission Results
Query the status or execution results of a submission using its token.

* **URL**: `/submissions/:token`
* **Method**: `GET`
* **Response Status**: `200 OK` (or `404 Not Found` if the token is invalid)

* **Response Body (While Processing)**:
```json
{
  "token": "79b32e60-84cf-4d92-8086-5386db49f9be",
  "status": {
    "id": 2,
    "description": "Processing"
  },
  "stdout": null,
  "stderr": null,
  "compile_output": null,
  "time_ms": null,
  "memory_kb": null,
  "exit_code": null
}
```

* **Response Body (Completed - Status 3)**:
```json
{
  "token": "79b32e60-84cf-4d92-8086-5386db49f9be",
  "status": {
    "id": 3,
    "description": "Accepted"
  },
  "stdout": "Hello Otter!\n",
  "stderr": "",
  "compile_output": "",
  "time_ms": 52,
  "memory_kb": 8120,
  "exit_code": 0
}
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/submissions/79b32e60-84cf-4d92-8086-5386db49f9be
```

---

## 6. Batch Submissions
Create multiple code execution tasks in a single request.

* **URL**: `/submissions/batch`
* **Method**: `POST`
* **Response Status**: `201 Created`

* **Request Body**:
```json
{
  "submissions": [
    {
      "language": "python",
      "source_code": "print('job A')"
    },
    {
      "language": "javascript",
      "source_code": "console.log('job B');"
    }
  ]
}
```

* **Response Body**:
```json
{
  "submissions": [
    {
      "token": "e9a31bc4-fa9a-41f2-870a-cc4c68832a81",
      "status": { "id": 1, "description": "Queued" },
      "stdout": null, "stderr": null, "compile_output": null, "time_ms": null, "memory_kb": null, "exit_code": null
    },
    {
      "token": "a8f2cd99-6e3e-4389-9a2c-d900bbcb1234",
      "status": { "id": 1, "description": "Queued" },
      "stdout": null, "stderr": null, "compile_output": null, "time_ms": null, "memory_kb": null, "exit_code": null
    }
  ]
}
```

* **Example Request**:
```bash
curl -X POST http://localhost:8080/submissions/batch \
  -H "Content-Type: application/json" \
  -d '{
    "submissions": [
      {"language": "python", "source_code": "print(1)"},
      {"language": "python", "source_code": "print(2)"}
    ]
  }'
```

---

## 7. Secured Observability Metrics
Returns the engine's dynamic run statistics, queue status, language usage, and run breakdowns.

* **URL**: `/admin/metrics`
* **Method**: `GET`
* **Response Status**: `200 OK` (requires authorization bearer token. For `/admin/*` routes, `OTTER_ADMIN_KEY` is preferred if configured, falling back to `OTTER_API_KEY` otherwise. Non-admin routes continue accepting either configured key.)
* **Response Body**:
```json
{
  "submissions": {
    "count": 150,
    "error_rate": 0.12,
    "avg_latency_ms": 115.4
  },
  "status_breakdown": {
    "accepted": 132,
    "compilation_error": 8,
    "time_limit_exceeded": 4,
    "memory_limit_exceeded": 3,
    "runtime_error": 3
  },
  "languages": {
    "python": 75,
    "javascript": 50,
    "c": 15,
    "cpp": 10
  },
  "queue": {
    "depth": 2,
    "in_flight": 4
  }
}
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/admin/metrics -H "Authorization: Bearer <your-key>"
```

---

## 8. Submission Status Reference

| Status ID | Description | Description & Condition |
| :---: | :--- | :--- |
| **`1`** | **`Queued`** | Job is waiting to be processed by a worker. |
| **`2`** | **`Processing`** | Job is compiling or running in the sandbox. |
| **`3`** | **`Accepted`** | Completed successfully with exit code 0. |
| **`4`** | **`Time Limit Exceeded`** | Exceeded CPU time limit (`RLIMIT_CPU`) or wall clock timeout limit. |
| **`5`** | **`Memory Limit Exceeded`** | Exceeded peak VSZ (`VmPeak`) or Peak RSS (`VmHWM`) limit. |
| **`6`** | **`Compilation Error`** | Compiler exited with non-zero exit status (compiler output returned in `compile_output`). |
| **`7`** | **`Runtime Error`** | Terminated by crash, non-zero code, or seccomp violation (`exit_code = 159`). |
| **`8`** | **`Internal Error`** | Failed to create sandbox folders or execute worker task. |

---

## 9. User Identity & Rate Limiting

Otter supports identity-aware rate limiting and execution fairness so backend services can propagate authenticated user identities rather than sharing a single IP quota.

### Authentication & Assertion Headers

| Header | Description |
|---|---|
| `Authorization: Bearer <key>` | Internal service API key (`OTTER_API_KEY` or `OTTER_ADMIN_KEY`). |
| `X-Otter-User-Assertion: <jwt>` | Signed backend assertion JWT identifying the end-user. Verified using `OTTER_JWT_SECRET`. |
| `X-Otter-User-Id: <user-id>` | Used in `OTTER_IDENTITY_MODE=trusted_header` when running behind a trusted private reverse proxy. |
| `X-Otter-Tenant-Id: <tenant-id>` | Optional tenant partition for multi-tenant rate limiting. |

#### JWT Assertion Claims
When using `X-Otter-User-Assertion`, Otter validates an HMAC-SHA256 JWT containing:
* `sub` (string, required): Stable user identifier (e.g., `usr_12345`).
* `exp` (integer, required): Expiration epoch seconds.
* `iss` (string, optional): Validated against `OTTER_JWT_ISSUER` if configured.
* `aud` (string, optional): Validated against `OTTER_JWT_AUDIENCE` if configured.
* `tenant_id` (string, optional): Groups quota under `tenant:<tenant>:user:<sub>`.

### Rate Limiting & Execution Concurrency

When rate limiting (`RATE_LIMIT_REQUESTS` and `RATE_LIMIT_WINDOW_SECONDS`) is enabled:
* Quotas are enforced against the derived identity (`user:<id>`, `key:<id>`, or `ip:<addr>`).
* When limits are exceeded, Otter returns `429 Too Many Requests` with the standard header:
  ```http
  Retry-After: 42
  ```
* Concurrent worker execution limits are enforced per user (`MAX_CONCURRENT_PER_USER`) and globally (`MAX_CONCURRENT`), preventing any individual user from exhausting worker capacity.

#### Creating the Assertion JWT (Backend Example)

To authenticate individual users through your backend, sign a short-lived token using your shared `OTTER_JWT_SECRET`:

**Node.js Example:**
```javascript
import jwt from 'jsonwebtoken';

function getUserAssertion(userId, tenantId = null) {
  return jwt.sign(
    {
      sub: userId,
      tenant_id: tenantId,
      exp: Math.floor(Date.now() / 1000) + 300, // 5 min TTL
    },
    process.env.OTTER_JWT_SECRET,
    { algorithm: 'HS256' }
  );
}
```

Attach the resulting token to your request:
```http
POST /submissions HTTP/1.1
Host: localhost:8080
Authorization: Bearer <OTTER_API_KEY>
X-Otter-User-Assertion: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
```
