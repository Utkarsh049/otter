# Otter — Integration & Setup Guide for Online IDEs

This guide explains how to architect, deploy, and securely integrate **Otter** as the sandboxed code-execution backend for an Online IDE or code-learning platform.

---

## 1. Architectural Blueprint

Otter is designed as an **internal execution microservice**. It should run in an isolated network segment and should **never** be exposed directly to public internet traffic or end-user browsers.

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           Client / Browser                              │
│              (React, Vue, Svelte, Monaco Editor, Xterm.js)              │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │ HTTPS / WSS
                                     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        Your Application Backend                         │
│                    (Node.js, Go, Python, Django, etc.)                  │
│                                                                         │
│  - Authenticates users & manages projects                               │
│  - Generates short-lived user assertion JWTs                            │
│  - Attaches secret API keys                                             │
│  - Relays job status to clients via WebSockets, SSE, or Polling         │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │ Internal Network (HTTP)
                                     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         Otter Sandbox Service                           │
│                         (Private Docker / VPC)                          │
│                                                                         │
│  - Enforces per-user concurrency (`MAX_CONCURRENT_PER_USER`)            │
│  - Isolates execution: Bubblewrap (bwrap) + Seccomp-BPF + rlimits       │
│  - Dispatches tasks via In-Memory or Redis Queue                        │
│  - Delivers results via Webhook or Polling endpoint                     │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Quickstart with Docker Compose

Running Otter with Redis inside an isolated Docker network is the recommended deployment pattern.

### `docker-compose.yml`

```yaml
version: '3.8'

services:
  # Otter Code Execution Engine
  otter:
    image: otter:latest
    build:
      context: .
      dockerfile: docker/Dockerfile
      target: runner
    restart: unless-stopped
    environment:
      - HOST=0.0.0.0
      - PORT=8080
      - APP_ENV=production
      - LOG_FORMAT=json
      
      # Resource & Queue Controls
      - MAX_CONCURRENT=8
      - MAX_CONCURRENT_PER_USER=2
      - MAX_CONCURRENT_PER_IP=2
      - MAX_QUEUE_DEPTH=100
      - CPU_LIMIT_MS=5000
      - WALL_LIMIT_MS=10000
      - MEMORY_LIMIT_MB=128
      - MAX_OUTPUT_BYTES=1048576

      # Security & Authentication
      - OTTER_API_KEY=change_this_to_a_long_random_secret_api_key
      - OTTER_ADMIN_KEY=change_this_to_a_long_random_admin_key
      - OTTER_IDENTITY_MODE=jwt
      - OTTER_JWT_SECRET=change_this_to_a_shared_jwt_signing_secret
      
      # Persistence & Queue
      - REDIS_URL=redis://redis:6379
      - ALLOW_LOOPBACK_WEBHOOKS=false
    # Bubblewrap requires user namespace or SYS_ADMIN capability
    cap_add:
      - SYS_ADMIN
    depends_on:
      - redis
    networks:
      - internal-net
    # NOTICE: Do NOT expose port 8080 to the host network in production.
    # Your backend container joins 'internal-net' and calls 'http://otter:8080'.

  # Redis Queue & State Store
  redis:
    image: redis:7-alpine
    restart: unless-stopped
    networks:
      - internal-net

networks:
  internal-net:
    driver: bridge
```

To build and start the service:
```bash
docker compose up -d --build
```

---

## 3. Communication Patterns: Polling vs. Webhooks

Otter natively supports both patterns out of the box. You do not need to change Otter's configuration—the mode is selected dynamically based on whether you provide `webhook_url` in the request body.

### Comparison

| Feature | Pattern A: Polling | Pattern B: Webhook + WebSocket/SSE |
| :--- | :--- | :--- |
| **Best For** | Simple backends, serverless/lambda architectures, CLI tools | Real-time IDEs, competitive programming platforms |
| **Network Requirement** | One-way: Backend calls Otter | Two-way: Backend calls Otter, Otter calls Backend |
| **Connection Overhead** | Repeated short-lived HTTP calls | Single persistent client connection, zero polling overhead |
| **Latency to Result** | Polling interval (typically 200–500ms) | Instantaneous upon job completion |

---

### Pattern A: Polling Implementation

In this pattern, your application backend sends the code to Otter and polls Otter's `GET /submissions/:token` endpoint until the job finishes.

#### Backend Flow (Node.js / Express Example)

```javascript
import express from 'express';
import fetch from 'node-fetch';
import jwt from 'jsonwebtoken';

const app = express();
app.use(express.json());

const OTTER_BASE_URL = process.env.OTTER_URL || 'http://otter:8080';
const OTTER_API_KEY = process.env.OTTER_API_KEY || 'change_this_to_a_long_random_secret_api_key';
const OTTER_JWT_SECRET = process.env.OTTER_JWT_SECRET || 'change_this_to_a_shared_jwt_signing_secret';

// Generates an Otter user assertion token
function generateAssertion(userId, tenantId = null) {
  return jwt.sign(
    {
      sub: userId,
      tenant_id: tenantId,
      exp: Math.floor(Date.now() / 1000) + 300, // 5 minutes validity
    },
    OTTER_JWT_SECRET,
    { algorithm: 'HS256' }
  );
}

app.post('/api/run', async (req, res) => {
  const { language, code, stdin } = req.body;
  const userId = req.user?.id || 'anonymous_user';

  try {
    // 1. Submit code to Otter (omit webhook_url to poll)
    const submitRes = await fetch(`${OTTER_BASE_URL}/submissions`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${OTTER_API_KEY}`,
        'X-Otter-User-Assertion': generateAssertion(userId),
      },
      body: JSON.stringify({
        language,
        source_code: code,
        stdin: stdin || '',
        cpu_time_limit_ms: 5000,
        memory_limit_mb: 128,
      }),
    });

    if (!submitRes.ok) {
      const errData = await submitRes.json();
      return res.status(submitRes.status).json(errData);
    }

    const { token } = await submitRes.json();

    // 2. Poll Otter until execution finishes
    const startTime = Date.now();
    const timeoutMs = 12000;

    while (Date.now() - startTime < timeoutMs) {
      await new Promise((r) => setTimeout(r, 250)); // 250ms interval

      const pollRes = await fetch(`${OTTER_BASE_URL}/submissions/${token}`, {
        headers: {
          'Authorization': `Bearer ${OTTER_API_KEY}`,
        },
      });

      if (pollRes.ok) {
        const result = await pollRes.json();
        // Status ID: 1 = Queued, 2 = Processing, >= 3 = Terminal (Accepted, TLE, etc.)
        if (result.status.id >= 3) {
          return res.json({
            status: result.status.description,
            stdout: result.stdout || '',
            stderr: result.stderr || '',
            compile_output: result.compile_output || '',
            time_ms: result.time_ms,
            memory_kb: result.memory_kb,
            exit_code: result.exit_code,
          });
        }
      }
    }

    return res.status(504).json({ error: 'Execution polling timed out' });
  } catch (err) {
    console.error('Execution error:', err);
    res.status(500).json({ error: 'Internal execution bridge error' });
  }
});
```

---

### Pattern B: Webhooks + WebSockets/SSE Implementation

In this pattern:
1. The browser connects to your backend via WebSocket.
2. The user clicks "Run". The backend submits the job with a `webhook_url` pointing to an internal endpoint on your backend.
3. Otter executes the code and `POST`s the final result to your backend webhook.
4. Your backend immediately pushes the output down the active WebSocket to the browser terminal.

#### Backend Flow (Node.js + WebSockets Example)

```javascript
import express from 'express';
import http from 'http';
import { WebSocketServer } from 'ws';
import jwt from 'jsonwebtoken';

const app = express();
app.use(express.json());

const server = http.createServer(app);
const wss = new WebSocketServer({ server, path: '/ws' });

// Store active WebSocket connections by submission token
const activeSockets = new Map();

wss.on('connection', (ws) => {
  ws.on('close', () => {
    for (const [token, clientWs] of activeSockets.entries()) {
      if (clientWs === ws) activeSockets.delete(token);
    }
  });
});

// Endpoint called by browser to trigger execution
app.post('/api/run-async', async (req, res) => {
  const { language, code, stdin } = req.body;
  const userId = req.user?.id || 'user_1';

  // Your backend's URL reachable by Otter inside the Docker/VPC network
  const webhookUrl = 'http://backend:3000/internal/webhooks/otter';

  const submitRes = await fetch('http://otter:8080/submissions', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${process.env.OTTER_API_KEY}`,
      'X-Otter-User-Assertion': generateAssertion(userId),
    },
    body: JSON.stringify({
      language,
      source_code: code,
      stdin: stdin || '',
      webhook_url: webhookUrl,
    }),
  });

  const { token } = await submitRes.json();
  res.json({ token });
});

// Endpoint called by Otter when execution is complete
app.post('/internal/webhooks/otter', (req, res) => {
  const result = req.body; // SubmissionResponse
  const { token, stdout, stderr, compile_output, status } = result;

  const clientWs = activeSockets.get(token);
  if (clientWs && clientWs.readyState === clientWs.OPEN) {
    clientWs.send(JSON.stringify({
      type: 'OUTPUT',
      status: status.description,
      stdout,
      stderr,
      compile_output,
      time_ms: result.time_ms,
      memory_kb: result.memory_kb,
    }));
    activeSockets.delete(token);
  }

  res.status(200).send('OK');
});

server.listen(3000, () => console.log('Backend listening on port 3000'));
```

---

## 4. Status Codes Reference

When querying submission results (either via Webhook or Polling), Otter returns standard competitive-programming status IDs:

| ID | Description | Meaning |
| :---: | :--- | :--- |
| `1` | `Queued` | Waiting in the worker queue |
| `2` | `Processing` | Currently running inside the sandbox |
| `3` | `Accepted` | Completed with exit code 0 |
| `4` | `Wrong Answer` | Non-zero exit code or execution mismatch |
| `5` | `Time Limit Exceeded` | Killed by CPU or wall-clock timeout |
| `6` | `Memory Limit Exceeded` | Exceeded configured virtual or physical RAM limit |
| `7` | `Runtime Error` | Crashed due to unhandled signal/exception (SIGSEGV, etc.) |
| `8` | `Internal Error` | Sandbox initialization or server infrastructure error |
| `11` | `Compilation Error` | Language compiler (gcc/g++) returned errors |

---

## 5. Production Security Checklist

Before launching Otter for untrusted user code in production:

1. **Network Boundary:**
   - Verify Otter port `8080` is not mapped to `0.0.0.0` on the public host.
   - Use internal Docker network bridges or private VPC subnets.
2. **Reverse Proxy Configuration:**
   - If placing Otter behind a reverse proxy (e.g. NGINX, Envoy), set `TRUSTED_PROXIES=10.0.0.1,172.18.0.1` so that only verified proxy IPs can forward `X-Forwarded-For` headers.
3. **Identity & Fair Sharing:**
   - Enable `OTTER_IDENTITY_MODE=jwt` and sign assertions on your application backend.
   - Set `MAX_CONCURRENT_PER_USER=2` to ensure no single user can exhaust the execution pool.
4. **SSRF Hardening:**
   - Set `ALLOW_LOOPBACK_WEBHOOKS=false` to ensure user webhooks cannot hit local services (`127.0.0.1`, `::1`, `169.254.169.254`).
5. **Runtime Capabilities:**
   - Ensure the container runtime provides `SYS_ADMIN` capability (or unprivileged user namespaces) so Bubblewrap can mount `/workspace` and isolate network/mount namespaces.
6. **Health & Metrics Monitoring:**
   - Check server health at `GET /health` (no authentication required).
   - Scrape internal metrics at `GET /admin/metrics` with `Authorization: Bearer <OTTER_ADMIN_KEY>`.
