# syntax=docker/dockerfile:1.7
#
# CraftCloud image: upstream OxiCloud (unmodified) + CraftCloud's own web frontend (Rust/egui built
# to wasm with Trunk) + the Craft web builds pinned in apps.toml (only in the `with-apps` target)
# + Caddy in front. No OxiCloud compile and no Node. See docs/ARCHITECTURE.md.
#
# Multi-arch without emulation: the build stage runs on $BUILDPLATFORM and produces
# platform-independent files; the final stage only COPYs onto the multi-arch upstream image.
#
#   docker buildx build --platform linux/amd64,linux/arm64 -t craftcloud:dev .                       (slim)
#   docker buildx build --platform linux/amd64,linux/arm64 --target with-apps -t craftcloud:dev-apps .

# Keep OXICLOUD_VERSION equal to the tag checked out in upstream/oxicloud: our frontend is written
# against that version's REST API (upstream/oxicloud/resources/gen/openapi.json).
ARG OXICLOUD_VERSION=0.10.0
ARG CADDY_VERSION=2.11.9
ARG ALPINE_IMAGE=alpine:3.24
ARG RUST_IMAGE=rust:1.97-slim-bookworm
# The web stage needs glibc 2.38 or newer: Trunk downloads a prebuilt wasm-bindgen, and its arm64
# build won't run on bookworm's glibc 2.36 (building the image on an arm64 machine failed there).
ARG WEB_RUST_IMAGE=rust:1.97-slim-trixie
ARG TRUNK_VERSION=0.21.14

# ─── CraftCloud web frontend: wasm, built once on the build machine (same for every platform) ────
FROM --platform=$BUILDPLATFORM ${WEB_RUST_IMAGE} AS web
ARG TRUNK_VERSION
# Cargo jobs inside the build; keep it modest on a shared machine.
ARG CARGO_BUILD_JOBS=6
# Cargo fetches the pinned git dependencies (the egui fork) with the git command line: its built-in
# git library can't fetch a commit by hash from GitHub, and it only worked from a warm cache.
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} CARGO_TERM_COLOR=never CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl brotli git && rm -rf /var/lib/apt/lists/* && \
    rustup target add wasm32-unknown-unknown && \
    arch="$(uname -m)" && \
    case "$arch" in \
      x86_64) sum=f2b4680cd239693a646a2795e4633c625328d7b2a044fbe749fa3a2fe9e7036b ;; \
      aarch64) sum=b1d8e60e454f7fc182d9a4d95d1506ffbae947d8ba90f8f6f02da93b60f980f9 ;; \
      *) echo "no trunk build for $arch" >&2; exit 1 ;; \
    esac && \
    curl -fsSL -o /tmp/trunk.tgz "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-${arch}-unknown-linux-gnu.tar.gz" && \
    echo "$sum  /tmp/trunk.tgz" | sha256sum -c - && \
    tar -xzf /tmp/trunk.tgz -C /usr/local/bin trunk && rm /tmp/trunk.tgz
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
COPY apps apps
COPY xtask xtask
# Downloads are cached; build output is not: a shared target cache can hand one checkout's compiled
# crates to another build (cargo fingerprints workspace crates without their path).
RUN --mount=type=cache,id=craftcloud-cargo-registry,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=craftcloud-cargo-git,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/trunk \
    cd apps/craftcloud-web && trunk build --release --locked --cargo-profile wasm-release && \
    mkdir -p /out && cp -r /src/dist/web /out/web && \
    find /out/web -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.html' -o -name '*.css' \) \
      -exec gzip -k -9 -n {} \; -exec brotli -k -q 11 {} \;

# ─── Craft web builds: download, verify sha256, unpack, precompress ───────────────────────────────
FROM --platform=$BUILDPLATFORM ${ALPINE_IMAGE} AS apps
RUN apk add --no-cache python3 brotli
COPY apps.toml /build/apps.toml
COPY scripts/fetch-apps /build/fetch-apps
RUN --mount=type=cache,target=/cache \
    python3 -I /build/fetch-apps /build/apps.toml /out/apps --cache /cache && \
    mkdir -p /out/craftcloud && mv /out/apps/apps.json /out/craftcloud/apps.json

# ─── craftcloud-admin: a static musl binary, cross-compiled on the build machine ─────────────────
# One stage instance per target platform (TARGETARCH differs); rust-lld links without a C toolchain.
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS admin
ARG TARGETARCH
ARG CARGO_BUILD_JOBS=6
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} CARGO_TERM_COLOR=never CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN apt-get update && apt-get install -y --no-install-recommends git && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl ;; \
      arm64) echo aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" >&2; exit 1 ;; \
    esac > /target && rustup target add "$(cat /target)"
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
COPY apps apps
COPY xtask xtask
RUN --mount=type=cache,id=craftcloud-cargo-registry,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=craftcloud-cargo-git,sharing=locked,target=/usr/local/cargo/git \
    t="$(cat /target)" && \
    RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes" cargo build --release --locked -p craftcloud-admin --target "$t" && \
    mkdir -p /out && cp "target/$t/release/craftcloud-admin" /out/

FROM caddy:${CADDY_VERSION}-alpine AS caddy

# ─── Runtime, slim: COPY only (no RUN), so every platform builds without emulation ───────────────
# craftcloud:<ver>. No Craft web apps inside; CRAFTCLOUD_WEB_APPS=on has nothing to serve here.
FROM diocrafts/oxicloud:${OXICLOUD_VERSION} AS slim
ARG OXICLOUD_VERSION
ARG CRAFTCLOUD_VERSION=dev

LABEL org.opencontainers.image.title="CraftCloud" \
      org.opencontainers.image.description="Self-hosted cloud for the Craft apps (based on OxiCloud)" \
      org.opencontainers.image.source="https://github.com/jonasbove/craftcloud" \
      org.opencontainers.image.version="${CRAFTCLOUD_VERSION}" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.base.name="docker.io/diocrafts/oxicloud:${OXICLOUD_VERSION}" \
      com.craftcloud.oxicloud.version="${OXICLOUD_VERSION}" \
      com.craftcloud.web-apps="none"

COPY --from=caddy /usr/bin/caddy /usr/local/bin/caddy
COPY --from=web /out/web/ /srv/craftcloud/web/
COPY --from=admin --chmod=755 /out/craftcloud-admin /usr/local/bin/craftcloud-admin
COPY docker/Caddyfile /etc/craftcloud/Caddyfile
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/craftcloud-entrypoint

ENV CRAFTCLOUD_PORT=8086 \
    CRAFTCLOUD_VERSION=${CRAFTCLOUD_VERSION}

EXPOSE 8086

# /ready checks OxiCloud and its database, through Caddy.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD wget -qO- "http://127.0.0.1:${CRAFTCLOUD_PORT}/ready" >/dev/null || exit 1

ENTRYPOINT ["craftcloud-entrypoint"]
CMD []

# ─── Runtime with the Craft web apps: craftcloud:<ver>-apps (`--target with-apps`) ───────────────
# Bundled but still off until CRAFTCLOUD_WEB_APPS=on.
FROM slim AS with-apps
LABEL com.craftcloud.web-apps="bundled"
COPY --from=apps /out/ /srv/craftcloud/

# ─── Default target: the slim image ───────────────────────────────────────────────────────────────
FROM slim AS default
