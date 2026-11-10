# syntax=docker/dockerfile:1
# Builds the single `tokens` binary (web/, migrations/ and docs/API.md are compiled into it).

FROM rust:1.99-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
COPY docs ./docs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/tokens /usr/local/bin/tokens

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
