default:
    @just --list

build:
    cargo build --workspace

release:
    cargo build --workspace --release

install: release
    cargo install --path crates/am-cli

test:
    cargo nextest run --workspace
    cargo test --workspace --doc

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets --fix --allow-dirty -- -D warnings

check: fmt clippy

check-pedantic:
    cargo clippy -p am-core --all-targets -- -W clippy::pedantic -D warnings

bench:
    cargo bench -p am-core --bench drift

bench-baseline:
    ./scripts/bench-gate.sh --save

bench-gate:
    ./scripts/bench-gate.sh

# cargo-llvm-cov needs llvm-cov and llvm-profdata on PATH.
# Override via env vars if your LLVM install is not on PATH:
#   LLVM_COV=/path/to/llvm-cov LLVM_PROFDATA=/path/to/llvm-profdata just coverage

coverage:
    cargo llvm-cov nextest --workspace

coverage-html:
    cargo llvm-cov nextest --workspace --html --output-dir coverage/

coverage-lcov:
    cargo llvm-cov nextest --workspace --lcov --output-path lcov.info

audit:
    cargo audit
