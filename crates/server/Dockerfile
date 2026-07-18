FROM rust:1.89-bookworm AS build
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --package world-at-war-server

FROM rust:1.89-bookworm AS development
WORKDIR /workspace
RUN apt-get update \
  && apt-get install --no-install-recommends -y curl \
  && rm -rf /var/lib/apt/lists/* \
  && cargo install cargo-watch --locked
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY data/space-cards ./data/space-cards
EXPOSE 8000
ENV BIND_ADDR=0.0.0.0:8000
CMD ["cargo", "watch", "--poll", "--watch", "Cargo.toml", "--watch", "Cargo.lock", "--watch", "crates", "--exec", "run --package world-at-war-server"]

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install --no-install-recommends -y curl \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --create-home --uid 10001 app
RUN mkdir -p /app/data/cache/space-track /app/data/cache/airports && chown -R app:app /app
COPY --from=build /workspace/target/release/world-at-war-server /usr/local/bin/world-at-war-server
COPY data/space-cards /app/data/space-cards
WORKDIR /app
USER app
EXPOSE 8000
ENV BIND_ADDR=0.0.0.0:8000
ENTRYPOINT ["world-at-war-server"]
