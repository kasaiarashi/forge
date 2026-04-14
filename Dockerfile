# ============================================================
# Forge VCS — Lean runtime images (no compilation)
#
# Expects pre-built artifacts in the build context:
#   dist/forge-server   — Linux amd64 binary
#   dist/forge-web      — Linux amd64 binary
#   dist/ui/            — Built React app (index.html + assets/)
#
# Build:
#   docker build --target forge-server -t forge-server .
#   docker build --target forge-web    -t forge-web .
# ============================================================

# ── forge-server ─────────────────────────────────────────────
FROM debian:bookworm-slim AS forge-server

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY dist/forge-server /usr/local/bin/forge-server
COPY docker/forge-server.toml /etc/forge/forge-server.toml

RUN chmod +x /usr/local/bin/forge-server && \
    mkdir -p /data

EXPOSE 9876
VOLUME ["/data"]

ENTRYPOINT ["forge-server"]
CMD ["--config", "/etc/forge/forge-server.toml", "--storage", "/data"]

# ── forge-web ────────────────────────────────────────────────
FROM debian:bookworm-slim AS forge-web

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY dist/forge-web /usr/local/bin/forge-web
COPY dist/ui/ /srv/forge-web/ui/
COPY docker/forge-web.toml /etc/forge/forge-web.toml

RUN chmod +x /usr/local/bin/forge-web

EXPOSE 3000

ENTRYPOINT ["forge-web"]
CMD ["--config", "/etc/forge/forge-web.toml"]
