# syntax=docker/dockerfile:1

ARG MW_IMAGE=ghcr.io/rmoriz/microwave-server:0.6.3

# The bootstrap CLI comes from the same image the server runs.
FROM ${MW_IMAGE} AS microwave

# Release build. The locale YAML and SPDX licence list are the only Rails files
# the crate reads at compile time.
FROM rust:1.94-trixie AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs askama.toml ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY public ./public
COPY supermarket-ruby/src/supermarket/config/locales ./supermarket-ruby/src/supermarket/config/locales
COPY supermarket-ruby/src/supermarket/app/assets/data/licenses.json ./supermarket-ruby/src/supermarket/app/assets/data/licenses.json
RUN cargo build --release --bins

# Bootstrap only needs a shell, curl, openssl and psql; it never serves traffic.
FROM debian:trixie-slim AS bootstrap
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl openssl postgresql-client \
    && rm -rf /var/lib/apt/lists/*
COPY containers/bootstrap.sh /usr/local/bin/bootstrap
COPY --from=microwave /usr/local/bin/microwave-ctl /usr/local/bin/microwave-ctl
RUN chmod +x /usr/local/bin/bootstrap
ENTRYPOINT ["/usr/local/bin/bootstrap"]

# The default (last) stage is the application image.
FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin supermarket

WORKDIR /app
COPY --from=builder /build/target/release/supermarket-rs /build/target/release/supermarket-worker /app/
COPY --from=builder /build/static /app/static
COPY --from=builder /build/public /app/public
RUN mkdir -p /app/public/system && chown -R supermarket:supermarket /app

USER supermarket
ENV PORT=3000
VOLUME /app/public/system
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=12 \
    CMD curl -fsS "http://127.0.0.1:${PORT}/api/v1/health" | grep -q '"status":"ok"' || exit 1
CMD ["/app/supermarket-rs"]
