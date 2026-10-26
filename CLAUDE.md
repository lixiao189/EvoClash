# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

EvoClash is a top-down 2D multiplayer shooter set in a post-apocalyptic wasteland (Rust, Bevy 0.20, lightyear 0.31 over WebTransport/netcode.io, axum matchmaker). The client runs only in the browser (wasm32). README.md (in Chinese) documents gameplay rules, CLI flags, HA/latency design and the release process.

## Git workflow (required)

Never commit or push directly to `main`. For every change:

1. Create a branch first.
2. Commit there and push the branch.
3. Open a PR against `main`.
4. Enable auto-fix and auto-merge on the PR.

Branch protection requires the single `CI success` check (it aggregates fmt, clippy (native + wasm), tests, cargo-deny, helm lint, and a Docker image build that runs the image and waits for a launched match to heartbeat).

## Commands

```bash
cargo build                                          # all crates; dev profile optimizes deps (Bevy is slow unoptimized)
cargo fmt --all --check                              # CI: rustfmt
cargo clippy --workspace --all-targets --locked -- -D warnings   # CI: warnings are errors
cargo clippy -p evoclash --target wasm32-unknown-unknown --locked -- -D warnings   # CI: the real client build
cargo test --workspace --locked                      # CI: tests
cargo test -p evoclash-game <test_name>              # one test in one crate (crates: evoclash-game, evoclash-server, evoclash, evoclash-matchmaker)
cargo deny --all-features check                      # CI: advisories / licenses / sources
helm lint --strict deploy/helm/evoclash               # CI: Helm chart
```

Run locally (needs `cargo install trunk`; `make web` adds the wasm32 target itself): `make run` builds the wasm client into `target/web` and starts `evoclash-matchmaker --web-dir target/web`, which serves it at `http://127.0.0.1:8080/play/` and launches match servers on demand (`make run LISTEN=host:port BOTS=n`, defaults 127.0.0.1:8080 and 6; local match-server output goes to `logs/`, cleared by `make clean-logs`). `make watch` rebuilds the client on change; `make check` runs the CI lints/tests. Page query params: `?name=` (required; index.html asks), `&lag=<ms>` simulates latency. A bare `evoclash-server --port 7000 --bots 4` still runs, but browsers can only reach a server through the matchmaker (they need its certificate hash). Server `--no-monsters` disables monsters.

The client crate still compiles natively so `cargo test`/clippy cover it (sprite preview tests), but its native `main` just exits; dead-code lints are relaxed natively and enforced by the wasm clippy run.

When asking the user to look at a client change, rebuild first (`make web`, then reload the page) — a stale build has caused confusion before.

## Architecture

Workspace crates:

- `crates/game` (`evoclash-game`) — shared by server and client: `protocol.rs` (replicated components, `PlayerInput`, `GameEvent` messages, channel registration in `ProtocolPlugin`), `sim.rs` (deterministic movement/combat rules), `map/` (seeded arena generator: types and `ArenaMap` in `mod.rs`, RNG and `Lot` helpers in `lot.rs`, block layouts in `prefabs.rs`) and `nav.rs` (pathfinding), `items/` (armor, pickups and loot rolls in `mod.rs`; `inventory.rs`: the replicated `Loadout` backpack — typed hotbar [gun, gun, melee, throwable], a 6-slot bag and an ammo pouch per `AmmoKind`, since guns hold no rounds; `weapons.rs`: every weapon is a type implementing `Weapon` plus `MeleeWeapon` (reach/arc hit test, which the client's slash draws exactly) or `RangedWeapon` (projectiles, ammo, `volley`), reached from the wire id via `WeaponKind::arm`/`spec`/`melee`/`ranged`), `net.rs` (tick rate, `PROTOCOL_ID`, netcode key loading), and `api` (matchmaker JSON contract) in `lib.rs`.
- `crates/server` (`evoclash-server`) — headless authoritative server, one process per match. Plugins: connections, gameplay, combat (lag compensation via position history), inventory (hotbar selection, `InventoryMove` messages from the B panel, pickups, death bags; `take` is the one function that grants any `Item`), environment (crates, exploding barrels, safes, radiation), bots, monsters, ops (heartbeat/drain).
- `crates/client` (`evoclash`) — browser client: input, local prediction (`predict.rs`), rendering, effects, HUD, session/reconnect (async HTTP via `ehttp`, WebTransport link). `index.html` + `Trunk.toml` are the web shell; the UI font is a bundled Noto Sans SC subset (`assets/README.md` explains regenerating it when adding Chinese strings outside GB2312).
- `crates/matchmaker` (`evoclash-matchmaker`) — seat reservation, signed connect tokens (`tokens.rs`), match registry fed by heartbeats (`registry.rs`), local process launching (`fleet.rs`; `--no-launch` only adopts externally started servers), and serving the web client at `/play/` (`--web-dir`, default `web/` next to the binary).

Key cross-cutting points:

- **Seeded arena.** Every match gets a new layout: `ArenaMap::generate(seed)` assembles a 5x3 grid of hand-authored block prefabs (randomly mirrored, with randomized sizes/doors/clutter) around the vault. The seed is `MatchState::map_seed`; the server regenerates `Arena` when an intermission ends, and clients regenerate from the replicated seed (`render/scenery.rs` rebuilds the scenery, `minimap.rs` repaints). Generation uses its own SplitMix RNG so it is bit-identical on wasm and native — never use `rand` or platform float functions in it. Tests in `map/tests.rs` and `nav.rs` check hundreds of seeds for wall overlaps, notched corners, doorless buildings and unreachable spots; scatter helpers in `Lot` keep a 100 px walkable gap between solids. Props (crates/barrels/safes) are spawned from `ArenaMap::props` as replicated entities and block movement via the `Obstacles` resource passed to `sim::step_movement`.
- **Shared determinism.** Movement must stay identical on both sides: the server runs `sim` authoritatively and the client re-runs the same functions for prediction/rollback of the local player. Change rules in `crates/game/src/sim.rs`, not separately in server or client.
- **Server tick.** 64 Hz `FixedUpdate`, ordered by the chained `SimSet` sets in `server/src/gameplay/mod.rs`: `Input → Simulate → Combat → Resolve → Flow`. Damage is queued (`DamageQueue`) and applied in `Resolve`; outgoing events are buffered in an `Outbox` and flushed in `Flow`. New gameplay systems should go into the right set.
- **Replication.** State replicates every 2 ticks (`REPLICATION_INTERVAL`). New replicated components must be registered in `ProtocolPlugin`. Bump `PROTOCOL_ID` in `net.rs` on any wire-format change so stale clients are rejected at handshake.
- **Own shots are client-drawn.** The client draws its own projectiles immediately; the server does not echo them back and decides hits itself.
- **WebTransport certificates.** Each server makes a 14-day self-signed cert at startup (`TlsIdentity` in `connections.rs`) and reports its SHA-256 in heartbeats; the matchmaker passes it to browsers in `JoinResponse.cert_digest` (a join to a just-launched match waits for the first heartbeat). Servers drain themselves after 12 days. Browsers only allow WebTransport from secure pages, so anything beyond localhost needs HTTPS in front of the matchmaker.
- **Matchmaker ↔ server.** Servers heartbeat every 2 s (players, `tick_us`, cert digest) to every matchmaker URL, expanding hostnames to all resolved addresses (headless Service → all replicas); a server missing heartbeats for ~6 s is dropped. Match processes outlive a matchmaker restart and are re-adopted via heartbeats. SIGTERM drains a server (finish the match, then exit).
- **Kubernetes.** The Helm chart `deploy/helm/evoclash` (check with `helm lint` / `helm template`) runs the matchmaker as a `--no-launch` Deployment and match servers as an Agones Fleet; `server/src/agones.rs` handles Ready/Health/self-Allocate/Shutdown over the SDK sidecar's REST API. Production needs `EVOCLASH_NETCODE_KEY` (64 hex chars) shared by matchmaker and servers; otherwise a dev key is used.
- **Players are LPC sprites.** `crates/client/assets/lpc-src/` vendors the Universal LPC generator layers we use; an ignored test bakes them into sheets in `assets/lpc/` (`cargo test -p evoclash -- --ignored bake_lpc_sheets`, `src/lpc/bake.rs`): one body sheet per armor, a white bandana sheet tinted with the player color, and the club (front/behind). `src/lpc/mod.rs` owns the sheet layout, which frame a pose shows (walk, club swing, hip-held gun = thrust upper body over walking legs, fall on death) and the gun grip per facing; `render/players.rs` draws them as texture atlases at one texture pixel per world pixel (guns too, `GUN_SCALE`), unlike the procedural sprites at `PIXEL`. Check poses with `EVOCLASH_POSES_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_poses`. Raiders are drawn the same way from `lpc/raider.png` (fixed outfit, red bandana) with a rifle (their real weapon isn't replicated). Dogs (LPC wolves, walk/run), ghouls (generator zombie) and, at `lpc::BIG_SCALE`, spitters (slime) and the behemoth (golem) are LPC sheets too (`lpc_look` in `render/monsters.rs`), as are crates, barrels (tinted red) and safes; roaches, crawlers, drones and spore pods stay procedural. The prop art is CC-BY-SA 4.0. The map is LPC too: the same bake writes ground (dirt, a 9-sliced gravel patch that frays into the dirt), sidewalk, road, wall faces (brick/metal 64 px tall = `WALL_HEIGHT`, rock 40), a wall top and two flat roofs from crops in `assets/lpc-src/map/`; `render/scenery.rs` tiles them at 1x and 9-slices the roofs (`lpc::ROOF_BORDER`). Sandbags, radiation zones, the vault floor and building interiors stay procedural. The camera applies a dusty `ColorGrading` (needs `Tonemapping::Linear`; `None` skips grading).
- **Sprite files.** Files under `crates/client/assets/lpc/` are copied by Trunk to `/play/assets/lpc/` (`index.html`) and loaded at runtime; `src/assets.rs` holds `PendingAssets`, and the session only joins a match once every registered handle has loaded (`assets_ready`). The art is CC-BY-SA and friends, not MIT/Apache: every file in `lpc/` and `lpc-src/` needs a row in `assets/CREDITS.md` (`tests/credits.rs` fails otherwise); see `assets/LICENSE-ART.md`. Shown in game at `/play/credits.html`.
- **Procedural art.** `client/src/textures/` paints every sprite at startup into a `Canvas` (`canvas.rs`; one file per sprite family, assembled into `Textures` by `build` in `mod.rs`) (drawn at 2x, nearest sampling, characters face +X). `#[ignore]` preview tests (never run by `cargo test` or CI, only with `-- --ignored`) write BMP contact sheets for checking art without running the game, e.g. `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_survivor` (see `textures/tests.rs` for others, e.g. `preview_city` for ground/roof/wall tiles and props).
- **Depth.** `render/` sorts by `depth(y)`; parts of one actor are layered with tiny `Z_STEP` offsets relative to the actor. Roofs are lifted by the wall height and sorted with the building's front wall, so actors inside are under the roof; `Indoors` hides anyone in a building the local player isn't in.

Workspace lints allow clippy `type_complexity` and `too_many_arguments` (normal for Bevy systems).

## Sub-agent team

Four project agents live in `.claude/agents/`. The main session is the lead: it talks to the user, splits the work, briefs agents, merges their results, and alone does git (branch/commit/PR).

| Agent | Model | Owns | Use for |
|---|---|---|---|
| `game-designer` | sonnet | README `## 玩法` (no code) | rules, numbers, balance → short spec |
| `server-dev` | sonnet | `crates/server`, `crates/matchmaker`, `crates/game`, `deploy/` | authoritative rules, protocol, bots, monsters, matchmaking |
| `client-dev` | sonnet | `crates/client` | rendering, sprites, effects, HUD, input, prediction |
| `game-tester` | haiku | `#[cfg(test)]` code only | fmt/clippy/tests, missing tests, failure triage |

Keep it cheap:

- Don't spawn for small things. One-file fixes, questions and quick reads stay in the main session; agents are for multi-file features or a long noisy step (a full CI run).
- Pipeline only as far as needed: `game-designer` only when rules/numbers are undecided → `server-dev` (shared `crates/game` first, since the client builds on it) → `client-dev` → `game-tester`. Server and client may run in parallel only when the protocol is already settled and they touch no shared file.
- Briefs are short and self-contained: goal, exact files/symbols, acceptance check, and the spec lines that matter — not the whole conversation. Agents already get this file, so don't repeat it.
- Follow up with `SendMessage` to the agent that has the context instead of spawning a fresh one.
- Agents run scoped, quiet checks (`-p <crate>`, `-q`, `--message-format=short`); the full `make check` runs once, in `game-tester`, before the PR.
- Reports are capped (≈15 lines, file:line refs, no code dumps); relay only what the user needs.

Avoid edit conflicts between agents:

- Split work by file ownership, not by feature step. Every brief lists the files the agent may edit; agents running at the same time never share a file. Agents edit only their files and put any change needed elsewhere in their report for the lead to apply.
- Hub files have one writer: `crates/game/src/protocol.rs` and `PROTOCOL_ID` in `net.rs`, plugin registration (`build` in `render/mod.rs`, `gameplay/mod.rs`, `monsters/mod.rs`, both `main.rs`/`config.rs`), `textures/mod.rs` (`Textures` fields and `build`), `Cargo.toml`/`Cargo.lock`, and docs (`CLAUDE.md`, `README.md`, `.claude/`). The lead (or one named agent) makes these edits first, before fanning out, so later agents build against a settled interface.
- Big areas are module directories (`client/src/render/`, `client/src/textures/`, `game/src/map/`, `server/src/gameplay/`, `server/src/monsters/`): `mod.rs` holds shared types, constants and the plugin; each child file is one concern (`use super::*;`, items `pub(super)`). Put new code in the child it belongs to, or a new child file, rather than growing `mod.rs`, so parallel agents land in different files.
- Each such directory's `tests.rs` (and other `#[cfg(test)]` code) belongs to `game-tester` while implementers run in parallel.
- Agents sharing this checkout may run together only with disjoint file sets. If overlap can't be avoided, run them one after another, or give each `isolation: "worktree"` and the lead merges the branches.
- When a file passes ~600 lines, or two tasks collided in it, split it into a module directory as above in its own refactor PR before the feature work.

Escalate the model per call (Agent `model: "opus"`), not by editing the agent files, when a task touches:

- `sim.rs` or client prediction/rollback — a mismatch doesn't fail the build, it shows up as rubber-banding under lag;
- lag compensation (`combat.rs` position history) or `SimSet` ordering (when damage and events land);
- map generation (`map/`, `nav.rs`) — must stay bit-identical across wasm/native and pass the seed sweeps;
- unfamiliar Bevy 0.20 / lightyear 0.31 APIs, where a weaker model guesses old signatures;
- one feature spanning protocol + server + client.

Alternatively the lead writes the interface/design for such work, a sonnet agent implements it, and the lead reviews the diff. Also escalate (don't retry) when an agent fails the same build or test twice, or comes back with a wrong or vague result — a few failed sonnet rounds cost more than one opus run. If `game-tester` misreads failures, rerun it on sonnet.

## Releases

Bump `workspace.package.version` in `Cargo.toml` and `version`/`appVersion` in `deploy/helm/evoclash/Chart.yaml`, merge it, then push a matching tag `vX.Y.Z` (tags containing `-` are pre-releases). `release.yml` checks that the tag matches both versions, builds the web client (the only release asset, `evoclash-web-vX.Y.Z.tar.gz`) and the Linux server/matchmaker binaries that only go into the image (with `web/` bundled), pushes `ghcr.io/lixiao189/evoclash-server`, then pushes the chart to `oci://ghcr.io/lixiao189/charts/evoclash`.
