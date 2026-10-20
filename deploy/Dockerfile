# syntax=docker/dockerfile:1
# Reef container image. Build from the repository root:
#   docker build -f deploy/Dockerfile -t yadvlabs/reef:dev .
#   docker buildx build --platform linux/amd64,linux/arm64 -f deploy/Dockerfile -t yadvlabs/reef:0.1.0 .
# The binary is cross-compiled on the build platform (cargo-zigbuild, static musl), so a
# multi-arch build never runs the Rust compiler under emulation.
#
# Run:  docker run -d -p 4920:4920 -p 4922:4922 -v reef-data:/var/lib/reef yadvlabs/reef
# Configuration: REEF_* variables (see deploy/reef.conf.example), or mount /etc/reef/reef.conf.
# State under /var/lib/reef (uid 4920). Ports 4920 (HTTP/S) and 4922 (SSH).

# ---- web UI --------------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:24-bookworm-slim AS web
WORKDIR /src/web
ARG PNPM_VERSION=12.14.0
RUN npm install -g "pnpm@${PNPM_VERSION}"
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# ---- binary, cross-compiled for TARGETARCH -------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:1.95-bookworm AS build
ARG TARGETARCH
ARG REEF_COMMIT=unknown
ARG REEF_VERSION=
ENV REEF_COMMIT=${REEF_COMMIT} REEF_VERSION=${REEF_VERSION}
RUN apt-get update \
 && apt-get install -y --no-install-recommends cmake pkg-config python3-pip \
 && rm -rf /var/lib/apt/lists/* \
 && pip3 install --break-system-packages --no-cache-dir cargo-zigbuild
RUN case "$TARGETARCH" in \
      amd64) t=x86_64-unknown-linux-musl ;; \
      arm64) t=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && rustup target add "$t" && echo "$t" > /rust-target
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY bin/ bin/
COPY crates/ crates/
COPY deploy/systemd/ deploy/systemd/
COPY --from=web /src/web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=reef-registry-${TARGETARCH} \
    --mount=type=cache,target=/usr/local/cargo/git,id=reef-git-${TARGETARCH} \
    --mount=type=cache,target=/src/target,id=reef-target-${TARGETARCH} \
    t=$(cat /rust-target) \
 && cargo zigbuild --locked --release --target "$t" --bin reef \
 && install -m 0755 "target/$t/release/reef" /reef

# ---- runtime -------------------------------------------------------------------------------
FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Reef" \
      org.opencontainers.image.description="GitHub-compatible source hosting: git, issues, pull requests, Actions, projects, wikis" \
      org.opencontainers.image.source="https://github.com/yadvlabs/reef" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
 && apt-get install -y --no-install-recommends git git-lfs openssh-client ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 4920 reef \
 && useradd --system --uid 4920 --gid reef --home-dir /var/lib/reef --shell /usr/sbin/nologin reef \
 && mkdir -p /etc/reef/conf.d /etc/reef/secrets /var/lib/reef /var/log/reef \
 && chown -R reef:reef /etc/reef/secrets /var/lib/reef /var/log/reef \
 && chmod 0700 /etc/reef/secrets
COPY --from=build /reef /usr/local/bin/reef
COPY deploy/reef.conf.example /etc/reef/reef.conf.example
USER reef
WORKDIR /var/lib/reef
VOLUME ["/var/lib/reef"]
EXPOSE 4920 4922
ENV REEF_DATA_DIR=/var/lib/reef
# Readiness for orchestrators: GET /readyz on 4920. The default config path is used when no
# file is mounted, so REEF_* variables alone are enough; with none at all Reef runs in
# standalone mode on SQLite and the first sign-up becomes the site administrator.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/reef"]
CMD ["serve"]
