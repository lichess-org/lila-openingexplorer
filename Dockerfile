FROM rust:1-trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev \
    liburing-dev \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

RUN python3 -m pip install --break-system-packages \
    chess \
    requests

WORKDIR /app
COPY . .

RUN cargo build --release

ENV EXPLORER_LOG=lila_openingexplorer=info

CMD ["./target/release/lila-openingexplorer"]
