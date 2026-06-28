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
| `CPU_LIMIT_MS` | Default CPU time limit per job | `5000` |
| `WALL_LIMIT_MS` | Default wall-clock execution time limit per job | `10000` |
| `MEMORY_LIMIT_MB` | Default memory limit per job | `128` |
| `MAX_OUTPUT_BYTES` | Cap on stdout/stderr output truncation (in bytes) | `1048576` (1MB) |
| `REDIS_URL` | Connection URL for Redis data persistence (V2) | None (Disabled) |
| `APP_ENV` | Mode of the application (e.g. `production`) | `development` |
| `LOG_FORMAT` | Format of tracing outputs (e.g. `json`) | `text` |
| `RATE_LIMIT_REQUESTS` | Allowed requests per window | None (Disabled) |
| `RATE_LIMIT_WINDOW_SECONDS` | Duration of the rate limit window | None (Disabled) |

> [!NOTE]
> **Reverse Proxies & Load Balancers**: When deployed behind a load balancer or reverse proxy (e.g. on Heroku, Render, Railway, or Cloudflare), the built-in rate limiter automatically parses the `X-Forwarded-For` and `X-Real-IP` headers to extract the true client IP address.

---

## 2. Local Docker Deployment
To run the server in a local containerized environment:

### Build the Docker Image
```bash
docker build -f docker/Dockerfile -t otter:latest .
```

### Run the Container
```bash
docker run -p 8080:8080 \
  -e MAX_CONCURRENT=4 \
  -e LOG_FORMAT=json \
  -e RATE_LIMIT_REQUESTS=100 \
  -e RATE_LIMIT_WINDOW_SECONDS=60 \
  otter:latest
```

---

## 3. Deploying to Heroku
Otter can be deployed to Heroku using the Docker/Container stack.

### 1. Configure Heroku App
Log in to the Heroku CLI and set the stack to container:
```bash
heroku login
heroku container:login
heroku create my-otter-engine
```

### 2. Prepare `heroku.yml`
Heroku uses a `heroku.yml` manifest file to build and run the Docker container. An example configuration:
```yaml
build:
  docker:
    web: docker/Dockerfile
run:
  web: /usr/local/bin/otter
```

### 3. Deploy App
Initialize git (if not already done) and push the code:
```bash
git add .
git commit -m "Deploying to Heroku"
git push heroku main
```
Heroku will automatically build the image using the manifest file and start the web process.

### 4. Scale and Configure Settings
Set any necessary production environment variables via CLI or Heroku Dashboard:
```bash
heroku config:set APP_ENV=production
heroku config:set LOG_FORMAT=json
heroku config:set RATE_LIMIT_REQUESTS=60
heroku config:set RATE_LIMIT_WINDOW_SECONDS=60
```
Your application will be live at `https://my-otter-engine.herokuapp.com`.

---

## 4. Deploying to Railway
Railway provides a simple, direct-from-git deployment model.

### 1. Link Repository
1. Log in to the [Railway Console](https://railway.app/).
2. Select **New Project** -> **Deploy from GitHub repo**.
3. Choose the `otter` repository.

### 2. Configure Service
1. Railway will automatically detect the root `Dockerfile` (or `docker/Dockerfile` depending on structure).
2. Go to **Settings** -> under **Build**, set the Dockerfile path to `docker/Dockerfile`.
3. Go to **Variables** -> Add environment variables (`APP_ENV=production`, `LOG_FORMAT=json`, etc.).
4. Click **Deploy**.

---

## 5. Deploying to Render
Render can deploy Docker-based applications as Web Services.

### 1. Create a New Web Service
1. Log in to the [Render Dashboard](https://render.com/).
2. Click **New** -> **Web Service**.
3. Connect your GitHub repository.

### 2. Configure Settings
1. Set the runtime environment to **Docker**.
2. Expand the **Advanced** section:
   - Set **Docker Path** to `docker/Dockerfile`.
   - Set **Health Check Path** to `/health` (allows Render to verify server startup before routing traffic).
   - Add environment variables under **Environment Variables**.
3. Select the appropriate instance plan (a minimum of 512MB RAM is recommended).
4. Click **Create Web Service**.

---

## 6. Deploying to DigitalOcean App Platform
DigitalOcean App Platform supports direct container builds.

### 1. Launch New App
1. Log in to the [DigitalOcean Cloud Console](https://cloud.digitalocean.com/).
2. Click **Apps** -> **Create App**.
3. Link your GitHub account and select the `otter` repository.

### 2. Configure Resources
1. Select the component to edit (defaults to Web Service).
2. Change the build source settings:
   - Verify that DO auto-detects the Dockerfile. Set the Dockerfile path explicitly to `docker/Dockerfile`.
3. Set the HTTP Port to `8080`.
4. Add environment variables under the **Environment Variables** tab.
5. Click **Next** and deploy the application.

---

## 7. Sandbox Troubleshooting (Unprivileged User Namespaces)
Because the sandbox utilizes `bubblewrap` (`bwrap`) to jail execution, the host kernel must support and allow unprivileged user namespaces.

To check if your host OS allows this, run:
```bash
sysctl kernel.unprivileged_userns_clone
```

If it returns `1`, unprivileged user namespaces are enabled.
If it returns `0`, you can temporarily enable it by running:
```bash
sudo sysctl -w kernel.unprivileged_userns_clone=1
```
To persist this setting, add `kernel.unprivileged_userns_clone=1` to `/etc/sysctl.conf`.

