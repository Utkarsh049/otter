# Stage 1: Build the Rust API application
FROM rust:slim-bookworm AS builder

WORKDIR /usr/src/otter
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libseccomp-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release

# Stage 2: Create the test runner image
FROM rust:slim-bookworm AS tester

WORKDIR /usr/src/otter
COPY . .

# Install all compilers, runtimes, and libraries needed for sandbox test execution
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc \
    g++ \
    python3-minimal \
    nodejs \
    bubblewrap \
    pkg-config \
    libseccomp-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

CMD ["cargo", "test"]

# Stage 3: Create the final production runner image
FROM debian:bookworm-slim AS runner

# Avoid interactive prompts
ENV DEBIAN_FRONTEND=noninteractive

# Install only the runtime engines, compilers, and libraries
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc \
    g++ \
    python3-minimal \
    nodejs \
    bubblewrap \
    libseccomp2 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Verify that all runtimes are installed successfully (build fails if any are missing)
RUN gcc --version \
    && g++ --version \
    && python3 --version \
    && node --version \
    && bwrap --version

# Copy the compiled release binary from builder
COPY --from=builder /usr/src/otter/target/release/otter /usr/local/bin/otter

# Create an unprivileged user to run the server
RUN useradd -m -s /bin/bash otter
USER otter

# Production environment defaults
ENV HOST=0.0.0.0
ENV PORT=8080
ENV APP_ENV=production
ENV LOG_FORMAT=json

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/otter"]
