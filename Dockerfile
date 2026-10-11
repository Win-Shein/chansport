# ---------- build stage ----------
FROM rust:bookworm AS builder
WORKDIR /app

# Cache dependencies (build once with a dummy main).
COPY Cargo.toml Cargo.lock ./
COPY schema.sql ./
RUN mkdir src && printf 'fn main() {}\n' > src/main.rs \
 && cargo build --release \
 && rm -rf src

# Real sources
COPY src ./src
COPY templates ./templates
RUN touch src/main.rs && cargo build --release

# ---------- runtime stage ----------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/myanbet365 /usr/local/bin/myanbet365
COPY --from=builder /app/templates ./templates

ENV TEMPLATES_DIR=/app/templates
ENV PORT=3001
ENV DATABASE_URL=sqlite:///data/chan.db

VOLUME ["/data"]
EXPOSE 3001

CMD ["myanbet365"]
