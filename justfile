default:
    @just --list

check:
    cargo clippy --workspace --all-targets -- -D warnings

build:
    cargo build --workspace

test:
    cargo test --workspace

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check
