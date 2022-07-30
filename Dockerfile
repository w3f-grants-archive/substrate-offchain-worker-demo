################################################################################

# podman build -f Dockerfile -t ocw_demo:dev -t ghcr.io/interstellar-network/ocw_demo:dev --volume ~/.cargo:/root/.cargo:rw --volume $(pwd)/target/release:/usr/src/app/target/release:rw .
# NOTE: it CAN work with Docker but it less than ideal b/c it can not reuse the host's cache
# NOTE: when dev/test: if you get "ninja: error: loading 'build.ninja': No such file or directory"
# -> FIX: find target/release/ -type d -name "*-wrapper-*" -exec rm -rf {} \;
# b/c docker build has no support for volume contrary to podman/buildah
# podman run -it --name ocw_demo --rm -p 127.0.0.1:9944:9944 --env RUST_LOG="warn,info,debug" ocw_demo:dev

FROM ghcr.io/interstellar-network/ci-images/ci-base-rust:dev as builder

WORKDIR /usr/src/app

# "error: 'rustfmt' is not installed for the toolchain '1.59.0-x86_64-unknown-linux-gnu'"
# nightly else: build/node-template-runtime-ccb50f07771c83ec/build-script-build: "Rust nightly not installed, please install it!"
# cf .github/workflows/rust.yml and compare!
RUN rustup component add rustfmt && \
    rustup install nightly && \
    rustup target add --toolchain nightly wasm32-unknown-unknown

# - lsb-release software-properties-common: prereq of "llvm.sh"
RUN apt-get update && apt-get install -y \
    lsb-release software-properties-common \
    && rm -rf /var/lib/apt/lists/*

# prereq of rocksys: LLVM & clang
# the script will by default install "PKG="clang-$LLVM_VERSION lldb-$LLVM_VERSION lld-$LLVM_VERSION clangd-$LLVM_VERSION""
# TODO customize, only install clang+llvm?
RUN bash -c "$(wget -O - https://apt.llvm.org/llvm.sh)"

COPY . .
# MUST select a specific crate else "error: found a virtual manifest at `/usr/src/app/Cargo.toml` instead of a package manifest"
# node/ is indeed the only executable
# MUST use "--locked" else Cargo.lock is ignored
RUN cargo install --locked --path node

################################################################################

FROM ubuntu:20.04

EXPOSE 9944

ENV APP_NAME node-template

ENV LD_LIBRARY_PATH=$LD_LIBRARY_PATH:/usr/local/lib

# - ca-certificates(+exec update-ca-certificates):
#   Thread 'tokio-runtime-worker' panicked at 'no CA certificates found', /usr/local/cargo/registry/src/github.com-1ecc6299db9ec823/hyper-rustls-0.22.1/src/connector.rs:45
#   cf https://github.com/paritytech/substrate/issues/9984
# TODO? instead cf https://rustrepo.com/repo/awslabs-aws-sdk-rust [17.]
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/$APP_NAME /usr/local/bin/$APP_NAME

# MUST use full path eg /usr/local/bin/node-template, else "Error: executable file `$APP_NAME` not found in $PATH"
# DO NOT use eg "sh -c $APP_NAME" b/c with it CMD is NOT passed to ENTRYPOINT!
ENTRYPOINT ["/usr/local/bin/node-template"]
# cf README: "IMPORTANT: you **MUST** use `--enable-offchain-indexing=1`"
# --ws-external, needed else can not connect from host, cf https://github.com/substrate-developer-hub/substrate-node-template/blob/main/docker-compose.yml
CMD ["--dev", "--tmp", "--enable-offchain-indexing=1", "--ws-external"]