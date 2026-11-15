# syntax=docker/dockerfile:1

# Server image (default target):
#
#   docker build -t lila-openingexplorer .
#
# Image with the indexing tools as well (pgn_to_parquet, parquet_to_positions,
# parquet_to_games, ...):
#
#   docker build --target tools -t lila-openingexplorer-tools .

FROM rust:1.99-trixie AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang libclang-dev \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
ARG TARGET_CPU=x86-64-v3
RUN printf '[target.x86_64-unknown-linux-gnu]\nrustflags = ["-Ctarget-cpu=%s"]\n' "$TARGET_CPU" \
    > "$CARGO_HOME/config.toml"
ENV CARGO_PROFILE_RELEASE_DEBUG=false
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .

FROM builder AS build-explorer
RUN cargo build --release --locked --bin explorer

FROM builder AS build-tools
RUN cargo build --release --locked --bins \
    && mkdir /out \
    && find target/release -maxdepth 1 -type f -executable -exec cp {} /out/ \;

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 explorer \
    && useradd --uid 10001 --gid explorer --no-create-home --shell /usr/sbin/nologin explorer
USER explorer:explorer
ENV EXPLORER_BIND=0.0.0.0:9002
EXPOSE 9002

FROM runtime AS tools
COPY --from=build-tools /out/ /usr/local/bin/

FROM runtime AS explorer
COPY --from=build-explorer /src/target/release/explorer /usr/local/bin/explorer
ENTRYPOINT ["explorer"]
CMD ["--help"]
