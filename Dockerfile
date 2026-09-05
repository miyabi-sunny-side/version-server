# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/ ./
RUN npm run build

FROM rust:1.96.0-bookworm AS chef
WORKDIR /app
COPY rust-toolchain.toml ./
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
# The recipe normalizes the local version; restore real manifests after cooking.
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app
RUN mkdir data && chown 10001:10001 data
COPY --from=backend /app/target/release/version-server /usr/local/bin/version-server
COPY --from=frontend /app/client/dist ./client/dist
ENV APP_BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
USER 10001:10001
ENTRYPOINT ["version-server"]
