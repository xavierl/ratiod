# syntax=docker/dockerfile:1

FROM node:24-alpine AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY clients ./clients
COPY --from=web /web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/ratiod /ratiod

FROM scratch
COPY --from=build /ratiod /ratiod
ENV RATIOD_BIND=0.0.0.0:5081 \
    RATIOD_DATA_DIR=/data \
    RATIOD_PEER_PORT=49200
EXPOSE 5081 49200
VOLUME ["/data"]
ENTRYPOINT ["/ratiod"]
