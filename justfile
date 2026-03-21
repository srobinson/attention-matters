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

# Homebrew LLVM paths (cargo-llvm-cov needs llvm-cov and llvm-profdata)
export LLVM_COV := env("LLVM_COV", "/opt/homebrew/Cellar/llvm/22.1.0/bin/llvm-cov")
export LLVM_PROFDATA := env("LLVM_PROFDATA", "/opt/homebrew/Cellar/llvm/22.1.0/bin/llvm-profdata")

coverage:
    cargo llvm-cov nextest --workspace

coverage-html:
    cargo llvm-cov nextest --workspace --html --output-dir coverage/

coverage-lcov:
    cargo llvm-cov nextest --workspace --lcov --output-path lcov.info

audit:
    cargo audit
