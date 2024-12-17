# Optimized for bookd.lichess.ovh:
# - Ubuntu 22.04 (jammy)
# - AMD Ryzen 5 PRO 3600 (znver2)

FROM docker.io/ubuntu:jammy AS builder

RUN apt-get update && apt-get upgrade --yes && apt-get install --yes git libssl-dev liburing-dev pkg-config wget software-properties-common gpg lsb-release make

# Rust
ADD --chmod=755 https://sh.rustup.rs/ rustup.sh
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
RUN ./rustup.sh -y --no-modify-path --profile minimal --default-toolchain 1.83.0 && rustc --version
ENV RUSTFLAGS="-Ctarget-cpu=znver2 -Clinker-plugin-lto -Clinker=clang-19 -Clink-arg=-fuse-ld=lld-19"
ENV JEMALLOC_SYS_WITH_MALLOC_CONF="abort_conf:true,background_thread:true,metadata_thp:auto,dirty_decay_ms:30000,muzzy_decay_ms:30000"

# Matching clang and lld
ADD --chmod=755 https://apt.llvm.org/llvm.sh llvm.sh
RUN ./llvm.sh 19 && apt-get update && apt-get install --yes clang-19 lld-19
ENV CC=/usr/bin/clang-19
ENV CXX=/usr/bin/clang++-19
ENV LD=/usr/bin/lld-19

# Prepare working directory
WORKDIR /lila-openingexplorer
COPY Cargo.toml Cargo.lock ./
COPY lila-openingexplorer ./lila-openingexplorer
COPY lila-openingexplorer-import ./lila-openingexplorer-import

# Run tests
RUN cargo --config net.git-fetch-with-cli=true fetch
RUN cargo test
RUN cargo bench --features lto

# Build optimized binaries
RUN cargo build --release --features lto

# Final image
FROM docker.io/ubuntu:jammy
RUN apt-get update && apt-get upgrade --yes
COPY --from=builder /lila-openingexplorer/target/release/lila-openingexplorer /usr/local/bin/
COPY --from=builder /lila-openingexplorer/target/release/import-lichess /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/lila-openingexplorer"]
