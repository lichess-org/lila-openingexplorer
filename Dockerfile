FROM rust:1-trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev \
    liburing-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release

ENV EXPLORER_LOG=lila_openingexplorer=info

CMD ["./target/release/lila-openingexplorer"]
