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

## 3b. List Submissions History
Returns a list of all active or recently executed submissions stored in the system.

* **URL**: `/submissions`
* **Method**: `GET`
* **Response Status**: `200 OK`
* **Response Body**:
```json
[
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
]
```

* **Example Request**:
```bash
curl -X GET http://localhost:8080/submissions
```

---

## 4. Get Submission Results
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

## 5. Batch Submissions
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

## 6. Secured Observability Metrics
Returns the engine's dynamic run statistics, queue status, language usage, and run breakdowns.

* **URL**: `/admin/metrics`
* **Method**: `GET`
* **Response Status**: `200 OK` (requires authorization bearer token if API key is configured)
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

## 7. Submission Status Reference

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
