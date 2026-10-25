# LPC sprites

Sprite sheets from the [Liberated Pixel Cup](https://lpc.opengameart.org/) family that the
client loads at runtime (`src/assets.rs`). Trunk copies this folder to `/play/assets/lpc/`.
They are baked from the layers in `../lpc-src/` by
`cargo test -p evoclash -- --ignored bake_lpc_sheets` (`src/lpc/bake.rs`); don't edit them by hand.

The art here is **not** under the code's MIT/Apache license: see `../LICENSE-ART.md`. Every
file must be listed in `../CREDITS.md` with its authors, source and license; `tests/credits.rs`
fails the build otherwise.
