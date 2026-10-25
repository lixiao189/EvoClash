# Art credits

Sprite art used by EvoClash, with its authors, sources and licenses. The art is not under the
code's MIT/Apache license; see `LICENSE-ART.md`. This file is shown in the game at
`/play/credits.html`.

Every file in `lpc/` must have a row below (checked by `crates/client/tests/credits.rs`).
Authors and licenses for LPC character pieces come from the generator's `CREDITS.csv`; for
tilesets, from the `CREDITS-*.txt` shipped with each pack.

## Files

### Shipped sheets (`lpc/`)

Baked from the sources below by `cargo test -p evoclash -- --ignored bake_lpc_sheets`
(`src/lpc/bake.rs`), which recolors the white base layers and stacks them.

| File | Made from | License |
|---|---|---|
| `lpc/survivor.png` | body, head, pants, boots, long-sleeve shirt, gloves, plain mask, square backpack (all `lpc-src/` below, every animation) | CC-BY-SA 3.0 |
| `lpc/survivor-scrap.png` | the same plus leather armour | CC-BY-SA 3.0 |
| `lpc/survivor-kevlar.png` | the same plus chainmail | CC-BY-SA 3.0 |
| `lpc/survivor-power.png` | the same plus plate armour | CC-BY-SA 3.0 |
| `lpc/survivor-team.png` | bandana | OGA-BY 3.0 |
| `lpc/club.png` | mace (walk, attack_slash, hurt) | OGA-BY 3.0 |
| `lpc/club-behind.png` | mace, behind-the-body parts | OGA-BY 3.0 |

| `lpc/ground-dirt.png` | `lpc-src/map/ground-dirt.png` fill tiles | CC-BY-SA 3.0 |
| `lpc/ground-gravel-patch.png` | `lpc-src/map/ground-gravel-blob.png` | CC-BY-SA 3.0 |
| `lpc/raider.png` | the survivor layers with leather armour, recolored, and the bandana in red | CC-BY-SA 3.0 |
| `lpc/sidewalk.png`, `lpc/road.png` | `lpc-src/map/sidewalk_terrain.png`, `base_road_tile.png`, `cracks_transparent.png` | CC0 |
| `lpc/wall-brick.png`, `lpc/wall-metal.png`, `lpc/wall-rock.png` | `lpc-src/map/wall-*.png`, top trim and baseboard | CC-BY-SA 3.0 |
| `lpc/wall-top.png`, `lpc/roof-tar.png`, `lpc/roof-concrete.png` | `lpc-src/map/roof-*.png` | CC-BY-SA 3.0 |

| `lpc/wolf.png`, `lpc/wolf-grey.png` | `lpc-src/creatures/wolf-*.png`, re-laid out by facing | OGA-BY 3.0 |
| `lpc/ghoul.png` | zombie body and head, pants, bandages (`lpc-src/`, walk) | CC-BY-SA 3.0 |
| `lpc/slime.png` | `lpc-src/creatures/slime.png` | CC-BY-SA 3.0 |
| `lpc/golem.png` | `lpc-src/creatures/golem-walk.png` | OGA-BY 3.0 |
| `lpc/crate.png`, `lpc/barrel.png`, `lpc/safe.png`, `lpc/safe-open.png` | `lpc-src/props/*.png` | CC-BY-SA 4.0 |

 the backpack is only offered under CC-BY-SA
3.0 / GPL 3.0; every other piece is used under the first license listed for it below.

### Map sources (`lpc-src/map/`)

Crops of the tilesets, so the repository doesn't carry the whole multi-megabyte sheets. Each
pack's own credits file is vendored next to them and lists every author it requires.

| File | Cropped from (x, y, w, h) | Pack and authors | License used (offered) |
|---|---|---|---|
| `lpc-src/map/wall-brick.png` | `walls.png` (128, 832, 128, 96) | [LPC] Walls, see `lpc-src/map/CREDITS-walls.txt` | CC-BY-SA 3.0 |
| `lpc-src/map/wall-cobble.png` | `walls.png` (256, 928, 128, 96) | [LPC] Walls, see `lpc-src/map/CREDITS-walls.txt` | CC-BY-SA 3.0 |
| `lpc-src/map/wall-corrugated.png` | `walls.png` (1152, 2464, 128, 96) | [LPC] Walls, see `lpc-src/map/CREDITS-walls.txt` | CC-BY-SA 3.0 |
| `lpc-src/map/roof-tar.png` | `roofs.png` (1112, 672, 112, 96) | [LPC] Roofs v2, see `lpc-src/map/CREDITS-roofs.txt` | CC-BY-SA 3.0 (CC-BY-SA 3.0, GPL 3.0) |
| `lpc-src/map/roof-concrete.png` | `roofs.png` (760, 672, 112, 96) | [LPC] Roofs v2, see `lpc-src/map/CREDITS-roofs.txt` | CC-BY-SA 3.0 (CC-BY-SA 3.0, GPL 3.0) |
| `lpc-src/map/ground-dirt.png` | `terrain-v7.png` (32, 96), (32, 160), (64, 160), (0, 160), 32x32 each | [LPC] Terrains, see `lpc-src/map/CREDITS-terrain.txt` | CC-BY-SA 3.0 (CC-BY-SA 3.0, 4.0) |
| `lpc-src/map/ground-gravel-blob.png` | `terrain-v7.png` (384, 64, 96, 96) | [LPC] Terrains, see `lpc-src/map/CREDITS-terrain.txt` | CC-BY-SA 3.0 (CC-BY-SA 3.0, 4.0) |
| `lpc-src/map/base_road_tile.png` | whole file | LPC Modern Streets by Faufilage | CC0 |
| `lpc-src/map/cracks_transparent.png` | whole file | LPC Modern Streets by Faufilage | CC0 |
| `lpc-src/map/sidewalk_terrain.png` | whole file | LPC Modern Streets by Faufilage | CC0 |
| `lpc-src/map/CREDITS-walls.txt` | the pack's credits | [LPC] Walls | — |
| `lpc-src/map/CREDITS-roofs.txt` | the pack's credits | [LPC] Roofs v2 | — |
| `lpc-src/map/CREDITS-terrain.txt` | the pack's credits | [LPC] Terrains | — |

### Creature and prop sources (`lpc-src/creatures/`, `lpc-src/props/`)

| File | Source | Authors | License used (offered) |
|---|---|---|---|
| `lpc-src/creatures/wolf-brown.png` | `wolfsheet1.png` of [LPC] Wolf Animation, <https://opengameart.org/content/lpc-wolf-animation> | Stephen "Redshrike" Challener (graphic artist), William.Thompsonj (contributor) | OGA-BY 3.0 (CC-BY 4.0, CC-BY 3.0, GPL 3.0, GPL 2.0, OGA-BY 3.0) |
| `lpc-src/creatures/wolf-grey.png` | `wolfsheet4.png` of [LPC] Wolf Animation, same page | Stephen "Redshrike" Challener, William.Thompsonj | OGA-BY 3.0 (as above) |
| `lpc-src/creatures/golem-walk.png` | [LPC] Golem, <https://opengameart.org/content/lpc-golem> | Stephen "Redshrike" Challener (graphic artist), William.Thompsonj (contributor) | OGA-BY 3.0 (CC-BY 4.0, CC-BY 3.0, GPL 3.0, GPL 2.0, OGA-BY 3.0) |
| `lpc-src/creatures/slime.png` | [LPC] Monsters, <https://opengameart.org/content/lpc-monsters> (also <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles>) | Charles Sanchez (CharlesGabriel), bagzie, bluecarrot16 | CC-BY-SA 3.0 (CC-BY-SA 3.0+, GPL 3.0+) |
| `lpc-src/props/crate.png` | `container.png` (64, 488, 32, 40) of [LPC] Containers v4.2, <https://opengameart.org/content/lpc-containers> | see `lpc-src/props/CREDITS-container.txt` | CC-BY-SA 4.0 |
| `lpc-src/props/barrel.png` | `container.png` (320, 0, 32, 44), same pack | see `lpc-src/props/CREDITS-container.txt` | CC-BY-SA 4.0 |
| `lpc-src/props/safe.png` | `container.png` (320, 256, 32, 32), same pack | see `lpc-src/props/CREDITS-container.txt` | CC-BY-SA 4.0 |
| `lpc-src/props/safe-open.png` | `container.png` (320, 320, 32, 32), same pack | see `lpc-src/props/CREDITS-container.txt` | CC-BY-SA 4.0 |
| `lpc-src/props/CREDITS-container.txt` | the pack's credits | [LPC] Containers | — |
| `lpc-src/body/bodies/zombie/walk/zombie.png` | Universal LPC generator (`body/bodies/zombie/walk.png`) | Stephen Challener (Redshrike), Johannes Sjölund (wulax), Sander Frenken (castelonia), Benjamin K. Smith (BenCreating), bluecarrot16 | OGA-BY 3.0 (OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) |
| `lpc-src/head/heads/zombie/adult/walk.png` | Universal LPC generator | bluecarrot16, Benjamin K. Smith (BenCreating), Sander Frenken (castelonia), Stephen Challener (Redshrike) | CC-BY-SA 3.0 (CC-BY-SA 3.0, GPL 3.0) |
| `lpc-src/torso/bandage/male/walk/white.png` | Universal LPC generator, <https://opengameart.org/content/lpc-bandages> | Nila122 | OGA-BY 3.0 (OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) |

The prop files (`lpc-src/props/` and the props they become in `lpc/`) are CC-BY-SA **4.0**:
[LPC] Containers is only offered under 4.0, and 4.0 art can't be relicensed as 3.0.

### Character sources (`lpc-src/`, from the Universal LPC Spritesheet Character Generator)

| File | Authors | License used (offered) | Sources |
|---|---|---|---|
| `lpc-src/body/bodies/male/walk.png` | bluecarrot16, JaidynReiman, Benjamin K. Smith (BenCreating), Evert, Eliza Wyatt (ElizaWy), TheraHedwig, MuffinElZangano, Durrani, Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-male-jumping-animation-by-durrani> <https://opengameart.org/content/lpc-runcycle-and-diagonal-walkcycle> <https://opengameart.org/content/lpc-revised-character-basics> <https://opengameart.org/content/lpc-be-seated> <https://opengameart.org/content/lpc-runcycle-for-male-muscular-and-pregnant-character-bases-with-modular-heads> <https://opengameart.org/content/lpc-jump-expanded> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/head/heads/human/male/walk.png` | bluecarrot16, Benjamin K. Smith (BenCreating), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/legs/pants/male/walk.png` | bluecarrot16, JaidynReiman, ElizaWy, Matthew Krohn (makrohn), Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, GPL 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-expanded-pants> |
| `lpc-src/feet/boots/basic/male/walk.png` | JaidynReiman, bluecarrot16, Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-clothes-and-hair> <https://opengameart.org/content/lpc-expanded-socks-shoes> |
| `lpc-src/torso/clothes/longsleeve/longsleeve/male/walk.png` | JaidynReiman, Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-revised-character-basics> <https://github.com/ElizaWy/LPC/tree/main/Characters/Clothing> <https://opengameart.org/content/lpc-expanded-sit-run-jump-more> <https://opengameart.org/content/lpc-expanded-simple-shirts> |
| `lpc-src/torso/armour/leather/male/walk.png` | Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-expanded-armor> |
| `lpc-src/torso/chainmail/male/walk.png` | Johannes Sjölund (wulax), Napsio (Vitruvian Studio), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> |
| `lpc-src/torso/armour/plate/male/walk.png` | Napsio (Vitruvian Studio), JaidynReiman, bluecarrot16, Michael Whitlock (bigbeargames), Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-combat-armor-for-women> |
| `lpc-src/arms/hands/gloves/male/walk.png` | Michael Whitlock (bigbeargames), Matthew Krohn (makrohn), Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> |
| `lpc-src/hat/cloth/bandana/adult/walk.png` | Matthew Krohn (makrohn), JaidynReiman, Marcel van de Steeg (MadMarcel), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-female-orcogregoblintroll-base-walkcycle> <https://github.com/makrohn/Universal-LPC-spritesheet/commit/f50007cb47c235d8896cafae7a613f0b6a9a09a8?short_path=02b86d4#diff-02b86d45789a3e3e8e79519c7d17d15c9e6ecc9b4ddecb1bcd8dfbbaef430b75> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/facial/masks/plain/adult/walk/charcoal.png` | Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-masks> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/backpack/squarepack/male/walk/forest.png` | Benjamin K. Smith (BenCreating), macmanmatty | CC-BY-SA 3.0 (offered: CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-backpacks> |
| `lpc-src/body/bodies/male/slash.png` | bluecarrot16, JaidynReiman, Benjamin K. Smith (BenCreating), Evert, Eliza Wyatt (ElizaWy), TheraHedwig, MuffinElZangano, Durrani, Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-male-jumping-animation-by-durrani> <https://opengameart.org/content/lpc-runcycle-and-diagonal-walkcycle> <https://opengameart.org/content/lpc-revised-character-basics> <https://opengameart.org/content/lpc-be-seated> <https://opengameart.org/content/lpc-runcycle-for-male-muscular-and-pregnant-character-bases-with-modular-heads> <https://opengameart.org/content/lpc-jump-expanded> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/head/heads/human/male/slash.png` | bluecarrot16, Benjamin K. Smith (BenCreating), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/legs/pants/male/slash.png` | bluecarrot16, JaidynReiman, ElizaWy, Matthew Krohn (makrohn), Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, GPL 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-expanded-pants> |
| `lpc-src/feet/boots/basic/male/slash.png` | JaidynReiman, bluecarrot16, Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-clothes-and-hair> <https://opengameart.org/content/lpc-expanded-socks-shoes> |
| `lpc-src/torso/clothes/longsleeve/longsleeve/male/slash.png` | JaidynReiman, Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-revised-character-basics> <https://github.com/ElizaWy/LPC/tree/main/Characters/Clothing> <https://opengameart.org/content/lpc-expanded-sit-run-jump-more> <https://opengameart.org/content/lpc-expanded-simple-shirts> |
| `lpc-src/torso/armour/leather/male/slash.png` | Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-expanded-armor> |
| `lpc-src/torso/chainmail/male/slash.png` | Johannes Sjölund (wulax), Napsio (Vitruvian Studio), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> |
| `lpc-src/torso/armour/plate/male/slash.png` | Napsio (Vitruvian Studio), JaidynReiman, bluecarrot16, Michael Whitlock (bigbeargames), Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-combat-armor-for-women> |
| `lpc-src/arms/hands/gloves/male/slash.png` | Michael Whitlock (bigbeargames), Matthew Krohn (makrohn), Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> |
| `lpc-src/hat/cloth/bandana/adult/slash.png` | Matthew Krohn (makrohn), JaidynReiman, Marcel van de Steeg (MadMarcel), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-female-orcogregoblintroll-base-walkcycle> <https://github.com/makrohn/Universal-LPC-spritesheet/commit/f50007cb47c235d8896cafae7a613f0b6a9a09a8?short_path=02b86d4#diff-02b86d45789a3e3e8e79519c7d17d15c9e6ecc9b4ddecb1bcd8dfbbaef430b75> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/facial/masks/plain/adult/slash/charcoal.png` | Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-masks> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/backpack/squarepack/male/slash/forest.png` | Benjamin K. Smith (BenCreating), macmanmatty | CC-BY-SA 3.0 (offered: CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-backpacks> |
| `lpc-src/body/bodies/male/thrust.png` | bluecarrot16, JaidynReiman, Benjamin K. Smith (BenCreating), Evert, Eliza Wyatt (ElizaWy), TheraHedwig, MuffinElZangano, Durrani, Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-male-jumping-animation-by-durrani> <https://opengameart.org/content/lpc-runcycle-and-diagonal-walkcycle> <https://opengameart.org/content/lpc-revised-character-basics> <https://opengameart.org/content/lpc-be-seated> <https://opengameart.org/content/lpc-runcycle-for-male-muscular-and-pregnant-character-bases-with-modular-heads> <https://opengameart.org/content/lpc-jump-expanded> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/head/heads/human/male/thrust.png` | bluecarrot16, Benjamin K. Smith (BenCreating), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/legs/pants/male/thrust.png` | bluecarrot16, JaidynReiman, ElizaWy, Matthew Krohn (makrohn), Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, GPL 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-expanded-pants> |
| `lpc-src/feet/boots/basic/male/thrust.png` | JaidynReiman, bluecarrot16, Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-clothes-and-hair> <https://opengameart.org/content/lpc-expanded-socks-shoes> |
| `lpc-src/torso/clothes/longsleeve/longsleeve/male/thrust.png` | JaidynReiman, Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-revised-character-basics> <https://github.com/ElizaWy/LPC/tree/main/Characters/Clothing> <https://opengameart.org/content/lpc-expanded-sit-run-jump-more> <https://opengameart.org/content/lpc-expanded-simple-shirts> |
| `lpc-src/torso/armour/leather/male/thrust.png` | Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-expanded-armor> |
| `lpc-src/torso/chainmail/male/thrust.png` | Johannes Sjölund (wulax), Napsio (Vitruvian Studio), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> |
| `lpc-src/torso/armour/plate/male/thrust.png` | Napsio (Vitruvian Studio), JaidynReiman, bluecarrot16, Michael Whitlock (bigbeargames), Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-combat-armor-for-women> |
| `lpc-src/arms/hands/gloves/male/thrust.png` | Michael Whitlock (bigbeargames), Matthew Krohn (makrohn), Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> |
| `lpc-src/hat/cloth/bandana/adult/thrust.png` | Matthew Krohn (makrohn), JaidynReiman, Marcel van de Steeg (MadMarcel), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-female-orcogregoblintroll-base-walkcycle> <https://github.com/makrohn/Universal-LPC-spritesheet/commit/f50007cb47c235d8896cafae7a613f0b6a9a09a8?short_path=02b86d4#diff-02b86d45789a3e3e8e79519c7d17d15c9e6ecc9b4ddecb1bcd8dfbbaef430b75> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/facial/masks/plain/adult/thrust/charcoal.png` | Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-masks> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/backpack/squarepack/male/thrust/forest.png` | Benjamin K. Smith (BenCreating), macmanmatty | CC-BY-SA 3.0 (offered: CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-backpacks> |
| `lpc-src/body/bodies/male/hurt.png` | bluecarrot16, JaidynReiman, Benjamin K. Smith (BenCreating), Evert, Eliza Wyatt (ElizaWy), TheraHedwig, MuffinElZangano, Durrani, Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-male-jumping-animation-by-durrani> <https://opengameart.org/content/lpc-runcycle-and-diagonal-walkcycle> <https://opengameart.org/content/lpc-revised-character-basics> <https://opengameart.org/content/lpc-be-seated> <https://opengameart.org/content/lpc-runcycle-for-male-muscular-and-pregnant-character-bases-with-modular-heads> <https://opengameart.org/content/lpc-jump-expanded> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/head/heads/human/male/hurt.png` | bluecarrot16, Benjamin K. Smith (BenCreating), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-character-bases> |
| `lpc-src/legs/pants/male/hurt.png` | bluecarrot16, JaidynReiman, ElizaWy, Matthew Krohn (makrohn), Johannes Sjölund (wulax), Stephen Challener (Redshrike) | OGA-BY 3.0 (offered: OGA-BY 3.0, GPL 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles> <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-expanded-pants> |
| `lpc-src/feet/boots/basic/male/hurt.png` | JaidynReiman, bluecarrot16, Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-clothes-and-hair> <https://opengameart.org/content/lpc-expanded-socks-shoes> |
| `lpc-src/torso/clothes/longsleeve/longsleeve/male/hurt.png` | JaidynReiman, Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-revised-character-basics> <https://github.com/ElizaWy/LPC/tree/main/Characters/Clothing> <https://opengameart.org/content/lpc-expanded-sit-run-jump-more> <https://opengameart.org/content/lpc-expanded-simple-shirts> |
| `lpc-src/torso/armour/leather/male/hurt.png` | Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-clothing-updates> <https://opengameart.org/content/lpc-expanded-armor> |
| `lpc-src/torso/chainmail/male/hurt.png` | Johannes Sjölund (wulax), Napsio (Vitruvian Studio), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> |
| `lpc-src/torso/armour/plate/male/hurt.png` | Napsio (Vitruvian Studio), JaidynReiman, bluecarrot16, Michael Whitlock (bigbeargames), Johannes Sjölund (wulax) | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-combat-armor-for-women> |
| `lpc-src/arms/hands/gloves/male/hurt.png` | Michael Whitlock (bigbeargames), Matthew Krohn (makrohn), Johannes Sjölund (wulax), bluecarrot16, JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <http://opengameart.org/content/lpc-clothing-updates> |
| `lpc-src/hat/cloth/bandana/adult/hurt.png` | Matthew Krohn (makrohn), JaidynReiman, Marcel van de Steeg (MadMarcel), JaidynReiman | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-female-orcogregoblintroll-base-walkcycle> <https://github.com/makrohn/Universal-LPC-spritesheet/commit/f50007cb47c235d8896cafae7a613f0b6a9a09a8?short_path=02b86d4#diff-02b86d45789a3e3e8e79519c7d17d15c9e6ecc9b4ddecb1bcd8dfbbaef430b75> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/facial/masks/plain/adult/hurt/charcoal.png` | Nila122 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0, GPL 2.0, GPL 3.0) | <https://opengameart.org/content/lpc-masks> <https://opengameart.org/content/lpc-expanded-hats-facial-helmets> |
| `lpc-src/backpack/squarepack/male/hurt/forest.png` | Benjamin K. Smith (BenCreating), macmanmatty | CC-BY-SA 3.0 (offered: CC-BY-SA 3.0, GPL 3.0) | <https://opengameart.org/content/lpc-backpacks> |
| `lpc-src/weapon/blunt/mace/walk/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |
| `lpc-src/weapon/blunt/mace/universal_behind/walk/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |
| `lpc-src/weapon/blunt/mace/attack_slash/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |
| `lpc-src/weapon/blunt/mace/attack_slash/behind/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |
| `lpc-src/weapon/blunt/mace/hurt/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |
| `lpc-src/weapon/blunt/mace/universal_behind/hurt/mace.png` | Johannes Sjölund (wulax), bluecarrot16 | OGA-BY 3.0 (offered: OGA-BY 3.0, CC-BY-SA 3.0) | <https://opengameart.org/content/lpc-medieval-fantasy-character-sprites> <https://opengameart.org/content/lpc-extended-weapon-animations> |

## Sources

- **Universal LPC Spritesheet Character Generator** — character layers.
  <https://github.com/LiberatedPixelCup/Universal-LPC-Spritesheet-Character-Generator>
  (per-file authors and licenses in its `CREDITS.csv`).
- **[LPC] Walls** by bluecarrot16 and contributors — CC-BY-SA 3.0.
  <https://opengameart.org/content/lpc-walls>
- **[LPC] Roofs** by bluecarrot16 and contributors — CC-BY-SA 3.0 / GPL 3.0.
  <https://opengameart.org/content/lpc-roofs>
- **[LPC] Terrains** by bluecarrot16 and contributors — CC-BY-SA 3.0 / 4.0.
  <https://opengameart.org/content/lpc-terrains>
- **LPC Modern Streets** by Faufilage — CC0. <https://opengameart.org/content/lpc-modern-streets>
- **LPC Shotgun Animation and Icon** — CC0.
  <https://opengameart.org/content/lpc-shotgun-animation-and-icon>

Liberated Pixel Cup art was originally commissioned by the Free Software Foundation, Creative
Commons and OpenGameArt.org (<https://lpc.opengameart.org/>).
