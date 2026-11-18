# CrewConvo server image (both editions). Build: docker build -t crewconvo .
FROM node:22-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates
COPY clients/python/crewconvo/SKILL.md ./clients/python/crewconvo/SKILL.md
COPY integrations ./integrations
RUN cargo build --release --bin crewconvo-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata locales \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 crewconvo \
    && mkdir -p /data && chown crewconvo:crewconvo /data
COPY --from=build /src/target/release/crewconvo-server /usr/local/bin/crewconvo-server
COPY --from=web /src/web/dist /app/web
# The built-in database of the Personal edition refuses to run as root.
USER crewconvo
WORKDIR /app
ENV DATA_DIR=/data \
    WEB_DIST=/app/web \
    BIND_ADDR=0.0.0.0:8090 \
    SANDBOX_ENABLED=false
VOLUME /data
EXPOSE 8090
CMD ["crewconvo-server"]
