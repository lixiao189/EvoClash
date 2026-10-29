# Local preview: `make run`, then open http://127.0.0.1:8080 in Chrome, Edge or Firefox.
# The matchmaker serves the wasm client and launches match servers on demand.

LISTEN ?= 127.0.0.1:8080
BOTS   ?= 8
# LAN=1: other machines can join. HTTP stays on LISTEN's port (bound on every interface);
# HTTPS defaults to 0.0.0.0:8443. PUBLIC_HOST overrides the detected LAN address.
# TLS_LISTEN overrides the HTTPS socket (must not be the HTTP port).
PUBLIC_HOST ?=
TLS_LISTEN ?=

CARGO_BINS := -p evoclash-server -p evoclash-matchmaker
WASM_TARGET := wasm32-unknown-unknown
# Empty for the debug client (cargo `dev` profile); `--release` uses the size-optimized `wasm` profile.
TRUNK_FLAGS := --release

.PHONY: run dev dev-lan web watch watch-dev server check clean-logs

## run: build everything and serve the game at http://$(LISTEN)
##      LAN=1 lets other machines on the network join (see `dev-lan`)
run: web server
	@echo "→ open http://$(LISTEN)"
	target/debug/evoclash-matchmaker --listen $(LISTEN) --bots $(BOTS) --web-dir target/web \
		$(if $(filter 1,$(LAN)),--lan,) \
		$(if $(PUBLIC_HOST),--public-host $(PUBLIC_HOST),) \
		$(if $(TLS_LISTEN),--tls-listen $(TLS_LISTEN),)

## dev: like `run`, but with a debug client: much faster to rebuild, slower in the browser
##      `make dev LAN=1` is the same as `dev-lan`
dev: TRUNK_FLAGS :=
dev: run

## dev-lan: `dev`, and other machines on the LAN can join
##           this machine: the http URL above; others: the https URL the matchmaker prints
##           (accept the self-signed certificate once). Override with PUBLIC_HOST= / TLS_LISTEN=
dev-lan: LAN := 1
dev-lan: dev

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
