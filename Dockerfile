# ---------- build stage ----------
FROM rust:bookworm AS builder
WORKDIR /app

# Cache dependencies (build once with a dummy main).
COPY Cargo.toml Cargo.lock ./
COPY schema.sql ./
RUN mkdir src && printf 'fn main() {}\n' > src/main.rs \
 && cargo build --release \
 && rm -rf src

# Real sources (templates are copied in the runtime stage so template-only
# changes don't invalidate the Rust build cache).
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ---------- runtime stage ----------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/myanbet365 /usr/local/bin/myanbet365
COPY templates ./templates

ENV TEMPLATES_DIR=/app/templates
ENV PORT=3000
ENV DATABASE_URL=sqlite:///data/chan.db
# Defaults (overridable by Coolify env vars at runtime).
ENV FIXTURES_API_URL=https://v3.football.api-sports.io/fixtures?date={date}
ENV FIXTURES_LEAGUE_IDS=39,140,78,135,61,40,2,3,848,1,4,9,5
ENV RESULT_SYNC_INTERVAL_MIN=180

VOLUME ["/data"]
EXPOSE 3000

CMD ["myanbet365"]
