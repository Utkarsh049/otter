# Otter — Deployment Guide

This document explains how to configure and deploy Otter in different environments.

---

## 1. Environment Variables Configuration

Create a `.env` file in the root of the project to manage environment variables:

| Variable Name | Description | Default Value |
| :--- | :--- | :--- |
| `HOST` | The binding IP address | `0.0.0.0` |
| `PORT` | The server port (dynamically assigned on Heroku/Render) | `8080` |
| `MAX_CONCURRENT` | Limit on background execution queue workers | `8` |
| `MAX_CONCURRENT_PER_IP` | Maximum concurrent sandboxes allowed per IP address | `2` |
| `MAX_CONCURRENT_PER_USER` | Maximum concurrent sandboxes allowed per user/identity | `2` |
| `MAX_QUEUE_DEPTH` | Maximum queue depth limit | `100` |
| `CPU_LIMIT_MS` | Default CPU time limit per job | `5000` |
| `WALL_LIMIT_MS` | Default wall-clock execution time limit per job | `10000` |
| `MEMORY_LIMIT_MB` | Default memory limit per job | `128` |
| `MAX_OUTPUT_BYTES` | Cap on stdout/stderr output truncation (in bytes) | `1048576` (1MB) |
| `DISABLE_SANDBOX` | Force raw fallback execution without Bubblewrap | `false` (Auto-detected) |
| `REDIS_URL` | Connection URL for Redis data persistence and distributed queues | None (Disabled) |
| `APP_ENV` | Mode of the application (e.g. `production`) | `development` |
| `LOG_FORMAT` | Format of tracing outputs (e.g. `json`) | `text` |
| `RATE_LIMIT_REQUESTS` | Allowed requests per client within rate limit window | None (Disabled) |
| `RATE_LIMIT_WINDOW_SECONDS` | Duration of the rate limit window in seconds | None (Disabled) |
| `OTTER_API_KEY` | Bearer API token(s) for service-to-service access | None (Disabled) |
| `OTTER_ADMIN_KEY` | Dedicated bearer API key for `/admin/*` routes (metrics & history) | None (Disabled) |
| `OTTER_IDENTITY_MODE` | User identity mode: `jwt`, `trusted_header`, or unset | None (Disabled) |
| `OTTER_JWT_SECRET` | Shared secret key for verifying user assertion JWTs | None (Disabled) |
| `OTTER_JWT_ISSUER` | Expected `iss` claim in user assertion JWTs | None (Disabled) |
| `OTTER_JWT_AUDIENCE` | Expected `aud` claim in user assertion JWTs | None (Disabled) |
| `TRUSTED_PROXIES` | Comma-separated list of trusted reverse proxy IPs | None (Disabled) |
| `ALLOW_LOOPBACK_WEBHOOKS` | Enable loopback webhooks (strictly `127.0.0.1`, `::1` for tests) | `false` |

---

## 2. Authentication, User Identity & Fair Sharing

### A. Service-to-Service Protection: `OTTER_API_KEY` vs. `OTTER_ADMIN_KEY`

* **`OTTER_API_KEY`**: Authenticates your main web application backend. Only clients providing `Authorization: Bearer <OTTER_API_KEY>` can submit code jobs or poll results.
* **`OTTER_ADMIN_KEY`**: A separate, higher-privileged key specifically for administrative endpoints (`/admin/metrics`, `/admin/submissions`). Use this key for monitoring tools (like Prometheus or your admin dashboard) while keeping normal execution clients restricted to `OTTER_API_KEY`.

### B. Solving the "Shared Backend IP" Problem: `OTTER_JWT_SECRET`

When an online IDE or web service connects to Otter, all requests arrive from the **same IP address** (your application backend). If Otter only throttled by IP address, a single active user could exhaust the entire server's quota, blocking all other users on your platform.

By enabling `OTTER_IDENTITY_MODE=jwt`, your backend signs a small assertion JWT identifying the end-user and forwards it in the `X-Otter-User-Assertion` header.

* Otter verifies the signature using `OTTER_JWT_SECRET`.
* Otter enforces `MAX_CONCURRENT_PER_USER` independently for each user (`user:<id>`).
* Otter's rate limiter throttles each user independently, preventing "noisy neighbors" from starving other users.

#### Generating the User Assertion JWT (Backend Examples)

**Node.js / JavaScript (`jsonwebtoken`):**
```javascript
import jwt from 'jsonwebtoken';

function createOtterAssertion(userId, tenantId = null) {
  return jwt.sign(
    {
      sub: userId,                        // Required: Unique user ID
      tenant_id: tenantId,                // Optional: Organization/workspace ID
      exp: Math.floor(Date.now() / 1000) + 300, // Valid for 5 minutes
    },
    process.env.OTTER_JWT_SECRET,
    { algorithm: 'HS256' }
  );
}

// Forward to Otter:
// headers: {
//   'Authorization': `Bearer ${process.env.OTTER_API_KEY}`,
//   'X-Otter-User-Assertion': createOtterAssertion('user_123')
// }
```

**Python (`PyJWT`):**
```python
import os
import time
import jwt

def create_otter_assertion(user_id: str, tenant_id: str = None) -> str:
    payload = {
        "sub": user_id,
        "tenant_id": tenant_id,
        "exp": int(time.time()) + 300,
    }
    return jwt.encode(payload, os.environ["OTTER_JWT_SECRET"], algorithm="HS256")
```

### C. Trusted Proxies (`TRUSTED_PROXIES`)

When deployed behind a reverse proxy (e.g. NGINX, Cloudflare, AWS ALB), client IPs are forwarded in `X-Forwarded-For` or `X-Real-IP`. To prevent untrusted clients from spoofing their IP address, set:
```env
TRUSTED_PROXIES=10.0.0.1,172.18.0.1
```
Otter will only read forwarded IP headers if the immediate TCP peer matches one of the configured `TRUSTED_PROXIES`.

---

## 3. Local Docker Deployment

### Build the Production Image
```bash
docker build -f docker/Dockerfile --target runner -t otter:latest .
```

### Run the Container
```bash
docker run -p 8080:8080 --privileged \
  -e MAX_CONCURRENT=4 \
  -e LOG_FORMAT=json \
  -e RATE_LIMIT_REQUESTS=100 \
  -e RATE_LIMIT_WINDOW_SECONDS=60 \
  otter:latest
```
> [!IMPORTANT]
> **Why `--privileged`**: The secure sandbox uses `bubblewrap` to jail user code. Inside Docker, bubblewrap requires `SYS_ADMIN` capability (granted by `--privileged` or `--cap-add=SYS_ADMIN`) to create user, mount, and network namespaces. If run without this capability, Otter automatically detects the restriction and falls back to **un-jailed raw execution mode**.

### Run Tests in the Container
```bash
docker compose -f docker-compose.test.yml up --build --exit-code-from test-runner
```

---

## 4. Deploying to Heroku
Otter can be deployed to Heroku using the Docker/Container stack. Because Otter's multi-stage Dockerfile is located at `docker/Dockerfile`, deploy using either the `heroku.yml` manifest or the Heroku Container CLI:

### Option A: Using `heroku.yml` Manifest (Recommended for Git Deploys)
1. **Create `heroku.yml` in repository root**:
   ```yaml
   build:
     docker:
       web:
         dockerfile: docker/Dockerfile
         target: runner
   ```
2. **Configure and Deploy**:
   ```bash
   heroku login
   heroku create my-otter-engine
   heroku stack:set container
   git push heroku main
   ```

### Option B: Using Heroku Container CLI
Push and release the multi-stage image directly from your local terminal:
```bash
heroku login
heroku container:login
heroku create my-otter-engine
heroku container:push web --context-path . -f docker/Dockerfile
heroku container:release web
```

### Configure Environment Variables
```bash
heroku config:set APP_ENV=production
heroku config:set LOG_FORMAT=json
heroku config:set OTTER_API_KEY=your_secure_api_key
```

---

## 5. Deploying to Railway & Render

Both Railway and Render automatically detect `docker/Dockerfile`.
- Set **Docker Path** to `docker/Dockerfile`.
- Set **Health Check Path** to `/health`.
- Configure your environment variables (`OTTER_API_KEY`, `OTTER_JWT_SECRET`, etc.).

---

## 6. Sandbox Troubleshooting (Unprivileged User Namespaces)

Because the sandbox utilizes `bubblewrap` (`bwrap`) to jail execution, the host kernel must support unprivileged user namespaces.

To check if your host OS allows this, run:
```bash
sysctl kernel.unprivileged_userns_clone
```
If it returns `1`, namespaces are enabled. To enable temporarily:
```bash
sudo sysctl -w kernel.unprivileged_userns_clone=1
```

### Automatic Fallback on Restricted Platforms
On platforms where `CLONE_NEWUSER` is blocked at the hypervisor level, Otter automatically detects namespace support at startup:
* Otter **gracefully falls back to un-jailed raw mode** (executing processes on the host rather than inside `bwrap`).
* In raw fallback mode, Otter enforces `RLIMIT_CPU`, `RLIMIT_AS`, `RLIMIT_FSIZE`, `RLIMIT_NOFILE`, and low CPU priority using standard Unix system calls. Process limits (`RLIMIT_NPROC`) are excluded in unjailed mode to avoid constraining the host daemon process.
* To explicitly force raw fallback mode:
  ```env
  DISABLE_SANDBOX=true
  ```
