default:
    @just --list

build:
    cargo build --workspace

test:
    cargo test --workspace

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

audit:
    cargo audit

install:
    cargo install --path crates/am-cli
