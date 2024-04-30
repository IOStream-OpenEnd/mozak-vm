#!/bin/sh
# This script tests transaction bundling.

# Run native executions and build mozakvm binaries
cd examples/token && cargo run --release \
    --features="native" \
    --bin token-native \
    --target "$(rustc -vV | grep host | awk '{ print $2; }')" \
    && cargo build --bin tokenbin --release

cd ../wallet && cargo run --release \
    --features="native" \
    --bin wallet-native \
    --target "$(rustc -vV | grep host | awk '{ print $2; }')" \
    && cargo build --bin walletbin --release


# Run CLI
cd ../../
cargo run --bin mozak-cli -- bundle-transaction -vvv \
    --system-tape examples/token/out/token.tape.json
