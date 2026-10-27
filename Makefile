# Local preview: `make run`, then open http://127.0.0.1:8080 in Chrome, Edge or Firefox.
# The matchmaker serves the wasm client and launches match servers on demand.

LISTEN ?= 127.0.0.1:8080
BOTS   ?= 8

CARGO_BINS := -p evoclash-server -p evoclash-matchmaker
WASM_TARGET := wasm32-unknown-unknown
# Empty for the debug client (cargo `dev` profile); `--release` uses the size-optimized `wasm` profile.
TRUNK_FLAGS := --release

.PHONY: run dev web watch watch-dev server check clean-logs

## run: build everything and serve the game at http://$(LISTEN)
run: web server
	@echo "→ open http://$(LISTEN)"
	target/debug/evoclash-matchmaker --listen $(LISTEN) --bots $(BOTS) --web-dir target/web

## dev: like `run`, but with a debug client: much faster to rebuild, slower in the browser
dev: TRUNK_FLAGS :=
dev: run

## web: build the browser client into target/web (needs `trunk`: cargo install trunk)
web:
	@rustup target list --installed | grep -qx $(WASM_TARGET) || rustup target add $(WASM_TARGET)
	cd crates/client && trunk build $(TRUNK_FLAGS)

## watch: rebuild the browser client on every change (run `make run` in another terminal)
watch:
	cd crates/client && trunk watch $(TRUNK_FLAGS)

## watch-dev: `watch` for the debug client (pair with `make dev`)
watch-dev: TRUNK_FLAGS :=
watch-dev: watch

## server: build the match server and matchmaker
server:
	cargo build $(CARGO_BINS)

## check: what CI runs (fmt, clippy native + wasm, tests)
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo clippy -p evoclash --target $(WASM_TARGET) --locked -- -D warnings
	cargo test --workspace --locked

clean-logs:
	rm -rf logs
