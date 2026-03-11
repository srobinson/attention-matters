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

install:
    cargo install --path crates/am-cli

# Frontend (chat/)
chat-install:
    cd chat && bun install

chat-dev:
    cd chat && bun run dev

chat-build:
    cd chat && bun run build

chat-lint:
    cd chat && bun run lint

chat-check:
    cd chat && bunx tsc --noEmit
