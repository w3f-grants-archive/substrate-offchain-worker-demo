# Substrate Off-chain Worker Demo

This repository is built based on [Substrate Node Template `v3.0.0+monthly-2021-10`](https://github.com/substrate-developer-hub/substrate-node-template/tree/v3.0.0+monthly-2021-10).

The purpose is to demonstrate what off-chain worker could do, and how one would go about using it.

### Run

1. First, complete the [basic Rust setup instructions](./docs/rust-setup.md).

2. To run it, use Rust's native `cargo` command to build and launch the template node:

  ```sh
  cargo run --release -- --dev --tmp --enable-offchain-indexing=1
  ```

  IMPORTANT: you **MUST** use `--enable-offchain-indexing=1` else it will always do nothing and show "[ocw-garble] nothing to do, returning..." and "[ocw-circuits] nothing to do, returning..." in the logs

3. To build it, the `cargo run` command will perform an initial build. Use the following command to
build the node without launching it:

  ```sh
  cargo build --release
  ```

Since this repository is based on Substrate Node Template,
[it's README](https://github.com/substrate-developer-hub/substrate-node-template/blob/v3.0.0%2Bmonthly-2021-10/README.md)
applies to this repository as well.

### Dev

For faster iteration:
- [MUST] use `lld` or `mold` as linker
- [not really needed] use Rust 1.62+
- [optional] `SKIP_WASM_BUILD=` allows to shave off ~10s; but CHECK wasm when adding dependencies!

NOTE: MUST use env var b/c using config.toml results in:
```
# mold or /usr/local/bin/mold :
#  Compiling wasm-test v1.0.0 (/tmp/.tmpH8Fteg)
#   error: linking with `rust-lld` failed: exit status: 1
# "note: rust-lld: error: unknown argument: -fuse-ld=/usr/local/bin/mold"
```
(both with lld and mold)

#### comparison

- [stable 1.60 + ld + wasm] `cargo build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 57.99s"
- [stable 1.60 + ld + NO wasm] `SKIP_WASM_BUILD= cargo build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 46.37s"
    - b/c remove ~10+s taken by "node-template-runtime v3.0.0-monthly-2021-10 build script (run) 	11.8s 		default, std"
- [stable 1.60 + lld] `SKIP_WASM_BUILD= RUSTFLAGS="-C linker=clang -C link-args=-fuse-ld=lld" cargo build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 19.79s"
- [stable 1.60 + mold] `SKIP_WASM_BUILD= RUSTFLAGS="-C linker=clang -C link-args=-fuse-ld=mold" cargo build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 19.49s"

- [nightly 1.62 + ld] `SKIP_WASM_BUILD= cargo +nightly build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 43.30s"
- [nightly 1.62 + lld] `SKIP_WASM_BUILD= RUSTFLAGS="-C link-args=-fuse-ld=lld" cargo +nightly build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 22.36s"
- [nightly 1.62 + mold] `SKIP_WASM_BUILD= RUSTFLAGS="-C linker=clang -C link-args=-fuse-ld=mold" cargo +nightly build --timings`
    - "Finished dev [unoptimized + debuginfo] target(s) in 17.95s"

### About Off-chain Worker

- The core of OCW features are demonstrated in [`pallets-ocw`](./pallets/ocw/src/lib.rs), and
[`pallets-example-offchain-worker`](./pallets/example-offchain-worker/src/lib.rs).

  Note that in order for the offchain worker to run, we have injected *Alice* key in
[`node/service.rs`](node/src/service.rs#L93-L104)

- Goto [**docs/README.md**](docs/README.md) to learn more about off-chain worker (extracted from Substrate
  Recipes, based on Substrate v3).

- Review the code of [**Offchain Worker Example Pallet** within Substrate](https://paritytech.github.io/substrate/latest/src/pallet_example_offchain_worker/lib.rs.html#18-709)
  and its [rustdoc](https://paritytech.github.io/substrate/latest/pallet_example_offchain_worker/).
  This pallet is also [added in this repository](pallets/example-offchain-worker).
