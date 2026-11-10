# syntax=docker/dockerfile:1
# Builds the server binary `tokens` from crates/server (its web/, migrations/ and docs/API.md are
# compiled into it).

FROM rust:1.99-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p tokens && cp target/release/tokens /usr/local/bin/tokens

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home --shell /usr/sbin/nologin tokens
COPY --from=build /usr/local/bin/tokens /usr/local/bin/tokens
USER tokens
ENV BIND=0.0.0.0:8787
EXPOSE 8787
ENTRYPOINT ["tokens"]
CMD ["serve"]
