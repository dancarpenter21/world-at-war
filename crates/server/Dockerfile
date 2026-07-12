FROM rust:1.89-bookworm AS build
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --package world-at-war-server

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 app
RUN mkdir -p /app/data/cache/space-track && chown -R app:app /app
COPY --from=build /workspace/target/release/world-at-war-server /usr/local/bin/world-at-war-server
WORKDIR /app
USER app
EXPOSE 8000
ENV BIND_ADDR=0.0.0.0:8000
ENTRYPOINT ["world-at-war-server"]
