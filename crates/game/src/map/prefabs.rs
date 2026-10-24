//! Hand-authored block prefabs.

use super::*;

/// The hand-authored block layouts. Each one randomizes sizes, doors and clutter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Prefab {
    Vault,
    Homes,
    Shops,
    Apartments,
    Warehouse,
    TankFarm,
    Rocks,
    Camp,
    RadLab,
    GasStation,
    Hospital,
    Checkpoint,
    Supermarket,
}

impl Prefab {
    pub(super) fn biome(self) -> Biome {
        match self {
            Prefab::Vault => Biome::Vault,
            Prefab::Homes
            | Prefab::Shops
            | Prefab::Apartments
            | Prefab::GasStation
            | Prefab::Hospital
            | Prefab::Supermarket => Biome::Urban,
            Prefab::Warehouse | Prefab::TankFarm | Prefab::Checkpoint => Biome::Industrial,
            Prefab::Rocks | Prefab::Camp => Biome::Wasteland,
            Prefab::RadLab => Biome::Radiation,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Prefab::Vault => "前文明地堡",
            Prefab::Homes => "住宅区",
            Prefab::Shops => "商业街",
            Prefab::Apartments => "公寓废墟",
            Prefab::Warehouse => "仓库区",
            Prefab::TankFarm => "储油罐区",
            Prefab::Rocks => "乱石岗",
            Prefab::Camp => "掠夺者营地",
            Prefab::RadLab => "辐射实验室",
            Prefab::GasStation => "加油站",
            Prefab::Hospital => "医院",
            Prefab::Checkpoint => "军事检查站",
            Prefab::Supermarket => "超市",
        }
    }

    pub(super) fn build(self, lot: &mut Lot) {
        match self {
            Prefab::Vault => vault(lot),
            Prefab::Homes => homes(lot),
            Prefab::Shops => shops(lot),
            Prefab::Apartments => apartments(lot),
            Prefab::Warehouse => warehouse(lot),
            Prefab::TankFarm => tank_farm(lot),
            Prefab::Rocks => rocks(lot),
            Prefab::Camp => camp(lot),
            Prefab::RadLab => rad_lab(lot),
            Prefab::GasStation => gas_station(lot),
            Prefab::Hospital => hospital(lot),
            Prefab::Checkpoint => checkpoint(lot),
            Prefab::Supermarket => supermarket(lot),
        }
    }
}

/// Outdoor odds and ends shared by most blocks.
pub(super) fn yard(lot: &mut Lot, crates: usize, barrels: usize, loot: usize) {
    let area = lot.area();
    let tier = lot.tier;
    for _ in 0..crates {
        lot.scatter_prop(area, PropKind::Crate, tier, true);
    }
    for _ in 0..barrels {
        lot.scatter_barrels(area, true);
    }
    for _ in 0..loot {
        lot.scatter_loot(area, tier);
    }
}

pub(super) fn wreck(lot: &mut Lot) {
    let area = lot.area();
    let size = if lot.chance(0.5) {
        v(112.0, 56.0)
    } else {
        v(56.0, 112.0)
    };
    lot.scatter_solid(area, size, WallStyle::Metal);
}

pub(super) fn rubble(lot: &mut Lot, count: usize) {
    let area = lot.area();
    for _ in 0..count {
        let size = v(lot.rand(50.0, 130.0), lot.rand(40.0, 100.0));
        lot.scatter_solid(area, size, WallStyle::Rock);
    }
}

/// The sealed bunker: four doorways, apex tech on a timer, information-age loot around it.
pub(super) fn vault(lot: &mut Lot) {
    let half = 220.0;
    let gap = 120.0;
    let seg = half - gap / 2.0;
    let th = 36.0;
    // Side walls run out past the corner by half a thickness so the corner block is solid.
    let side = seg + th / 2.0;
    for sign in [-1.0, 1.0] {
        lot.wall(
            v(-(gap / 2.0 + seg / 2.0), sign * half),
            v(seg, th),
            WallStyle::Brick,
        );
        lot.wall(
            v(gap / 2.0 + seg / 2.0, sign * half),
            v(seg, th),
            WallStyle::Brick,
        );
        lot.wall(
            v(sign * half, -(gap / 2.0 + side / 2.0)),
            v(th, side),
            WallStyle::Brick,
        );
        lot.wall(
            v(sign * half, gap / 2.0 + side / 2.0),
            v(th, side),
            WallStyle::Brick,
        );
    }
    for sign in [-1.0, 1.0] {
        lot.wall(v(0.0, sign * 360.0), v(160.0, 26.0), WallStyle::Sandbag);
        lot.wall(v(sign * 350.0, 0.0), v(26.0, 160.0), WallStyle::Sandbag);
    }
    for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        lot.loot(v(300.0 * sx, 330.0 * sy), 3);
    }
}

/// Two or three family houses with yards.
pub(super) fn homes(lot: &mut Lot) {
    let tier = lot.tier;
    let (w, h) = (lot.rand(240.0, 300.0), lot.rand(200.0, 240.0));
    let mut doors = vec![(Side::S, lot.rand(-40.0, 40.0))];
    if lot.chance(0.5) {
        doors.push((Side::E, 0.0));
    }
    let a = lot.building(
        v(-HALF.x + w / 2.0, HALF.y - h / 2.0),
        v(w, h),
        &doors,
        Roof::Tar,
        Floor::Planks,
    );
    lot.loot(a.center(), tier);
    if lot.chance(0.5) {
        let r = PropKind::Safe.radius() + 4.0;
        let p = lot.corner(a, r);
        lot.prop(p, PropKind::Safe, tier);
    }

    let (w, h) = (lot.rand(240.0, 300.0), lot.rand(200.0, 240.0));
    let mut doors = vec![(Side::N, lot.rand(-40.0, 40.0))];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    let b = lot.building(
        v(HALF.x - w / 2.0, -HALF.y + h / 2.0),
        v(w, h),
        &doors,
        Roof::Tin,
        Floor::Planks,
    );
    lot.loot(b.center(), tier);

    if lot.chance(0.6) {
        let side = if lot.chance(0.5) { Side::S } else { Side::W };
        let c = lot.building(
            v(HALF.x - 100.0, HALF.y - 90.0),
            v(200.0, 180.0),
            &[(side, 0.0)],
            Roof::Tar,
            Floor::Tiles,
        );
        lot.loot(c.center(), tier);
    }
    let area = lot.area();
    for _ in 0..2 {
        let size = if lot.chance(0.5) {
            v(lot.rand(80.0, 140.0), 24.0)
        } else {
            v(24.0, lot.rand(80.0, 140.0))
        };
        lot.scatter_solid(area, size, WallStyle::Sandbag);
    }
    wreck(lot);
    yard(lot, 3, 1, 1);
}

/// A strip of three shops facing a parking lot.
pub(super) fn shops(lot: &mut Lot) {
    let tier = lot.tier;
    let w = lot.rand(680.0, 740.0);
    let h = lot.rand(220.0, 250.0);
    let c = v(0.0, HALF.y - h / 2.0);
    let mut doors = vec![(Side::S, -w / 3.0), (Side::S, 0.0), (Side::S, w / 3.0)];
    if lot.chance(0.5) {
        doors.push((Side::N, 0.0));
    }
    let inside = lot.building(c, v(w, h), &doors, Roof::Tin, Floor::Tiles);
    for x in [c.x - w / 6.0, c.x + w / 6.0] {
        lot.run_v(inside.min.y, inside.max.y, x, &[]);
    }
    for x in [c.x - w / 3.0, c.x, c.x + w / 3.0] {
        lot.loot(v(x, c.y), tier);
    }
    if lot.chance(0.5) {
        let shop = Rect::new(
            c.x + w / 6.0 + WALL / 2.0,
            inside.min.y,
            inside.max.x,
            inside.max.y,
        );
        let r = PropKind::Safe.radius() + 4.0;
        lot.prop(v(shop.max.x - r, shop.max.y - r), PropKind::Safe, tier);
    }
    for _ in 0..3 {
        wreck(lot);
    }
    let area = lot.area();
    lot.scatter_solid(area, v(160.0, 26.0), WallStyle::Sandbag);
    yard(lot, 2, 2, 1);
}

/// A gutted apartment block: three rooms, a back door, maybe a hole in the wall.
pub(super) fn apartments(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(520.0, 580.0), lot.rand(460.0, 520.0));
    let c = v(0.0, 20.0);
    let h = size / 2.0;
    let split_x = c.x - 60.0;
    let split_y = c.y + 60.0;
    let mut doors = vec![
        (Side::S, -h.x / 2.0 - 30.0),
        (Side::E, (-h.y + WALL + (split_y - c.y) - 10.0) / 2.0),
        (Side::N, h.x / 2.0),
    ];
    if lot.chance(0.5) {
        doors.push((Side::W, lot.rand(-80.0, 80.0)));
    }
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Concrete);
    // Left wing | right wing, with a doorway low down.
    let gap_y = c.y - h.y / 2.0;
    lot.run_v(inside.min.y, inside.max.y, split_x, &[gap_y]);
    // Right wing split into two rooms.
    let x0 = split_x + WALL / 2.0;
    lot.run_h(x0, inside.max.x, split_y, &[(x0 + inside.max.x) / 2.0]);

    let left = Rect::new(
        inside.min.x,
        inside.min.y,
        split_x - WALL / 2.0,
        inside.max.y,
    );
    let low = Rect::new(x0, inside.min.y, inside.max.x, split_y - WALL / 2.0);
    let high = Rect::new(x0, split_y + WALL / 2.0, inside.max.x, inside.max.y);
    lot.loot(v(left.center().x, left.min.y + left.height() * 0.3), tier);
    lot.loot(v(left.center().x, left.min.y + left.height() * 0.75), tier);
    lot.loot(low.center(), tier);
    let next = lot.next_tier();
    lot.loot(high.center() - v(40.0, 0.0), next);
    let r = PropKind::Safe.radius() + 4.0;
    lot.prop(v(high.max.x - r, high.max.y - r), PropKind::Safe, tier);
    rubble(lot, 2);
    yard(lot, 2, 2, 0);
}

/// A big warehouse full of crates, shipping containers outside.
pub(super) fn warehouse(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(520.0, 600.0), lot.rand(320.0, 360.0));
    let c = v(
        lot.rand(-60.0, 60.0)
            .clamp(-HALF.x + size.x / 2.0, HALF.x - size.x / 2.0),
        HALF.y - size.y / 2.0,
    );
    let mut doors = vec![(Side::S, -size.x / 4.0), (Side::S, size.x / 4.0)];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    if lot.chance(0.5) {
        doors.push((Side::N, 0.0));
    }
    let inside = lot.building(c, size, &doors, Roof::Tin, Floor::Concrete);
    let inside = lot.world(inside);
    for _ in 0..4 {
        lot.scatter_prop(inside, PropKind::Crate, tier, false);
    }
    lot.scatter_loot(inside, tier);
    lot.scatter_loot(inside, tier);
    let area = lot.area();
    for _ in 0..3 {
        let size = if lot.chance(0.5) {
            v(200.0, 64.0)
        } else {
            v(64.0, 200.0)
        };
        lot.scatter_solid(area, size, WallStyle::Metal);
    }
    yard(lot, 1, 2, 1);
}

/// Fuel tanks, pipes and a little control shed. Lots of things that go boom.
pub(super) fn tank_farm(lot: &mut Lot) {
    let tier = lot.tier;
    let mut doors = vec![(Side::N, 0.0)];
    if lot.chance(0.5) {
        doors.push((Side::W, 0.0));
    }
    let shed = lot.building(
        v(HALF.x - 110.0, -HALF.y + 85.0),
        v(220.0, 170.0),
        &doors,
        Roof::Tin,
        Floor::Concrete,
    );
    lot.loot(shed.center(), tier);
    if lot.chance(0.4) {
        let r = PropKind::Safe.radius() + 4.0;
        let p = lot.corner(shed, r);
        lot.prop(p, PropKind::Safe, tier);
    }
    let area = lot.area();
    let tanks = 3 + lot.b.rng.below(2);
    for _ in 0..tanks {
        let s = lot.rand(110.0, 150.0);
        lot.scatter_solid(area, v(s, s), WallStyle::Metal);
    }
    for _ in 0..2 {
        let size = if lot.chance(0.5) {
            v(lot.rand(160.0, 240.0), 18.0)
        } else {
            v(18.0, lot.rand(160.0, 240.0))
        };
        lot.scatter_solid(area, size, WallStyle::Metal);
    }
    yard(lot, 2, 4, 2);
}

/// Open wasteland: boulders, wrecks and whatever fell off a truck.
pub(super) fn rocks(lot: &mut Lot) {
    let count = 8 + lot.b.rng.below(4);
    rubble(lot, count);
    wreck(lot);
    wreck(lot);
    yard(lot, 4, 2, 3);
}

/// A raider camp: a sandbag ring around a tent with the good stuff.
pub(super) fn camp(lot: &mut Lot) {
    let ring = 230.0;
    let len = 240.0;
    let open = lot.b.rng.below(6);
    for (i, (c, size)) in [
        (v(0.0, ring), v(len, 26.0)),
        (v(0.0, -ring), v(len, 26.0)),
        (v(ring, 0.0), v(26.0, len)),
        (v(-ring, 0.0), v(26.0, len)),
    ]
    .into_iter()
    .enumerate()
    {
        if i != open {
            lot.wall(c, size, WallStyle::Sandbag);
        }
    }
    let side = if lot.chance(0.5) { Side::N } else { Side::S };
    let tent = lot.building(
        v(0.0, 0.0),
        v(200.0, 150.0),
        &[(side, 0.0)],
        Roof::Tar,
        Floor::Planks,
    );
    let next = lot.next_tier();
    lot.loot(tent.center(), next);
    rubble(lot, 3);
    yard(lot, 3, 2, 1);
}

/// A crater-side lab soaked in radiation: the best loot outside the vault, at a price.
pub(super) fn rad_lab(lot: &mut Lot) {
    lot.hazard(Vec2::ZERO, HALF * 2.0);
    let size = v(lot.rand(300.0, 360.0), lot.rand(240.0, 280.0));
    let c = v(lot.rand(-40.0, 40.0), lot.rand(-40.0, 60.0));
    let doors = [
        (Side::S, lot.rand(-60.0, 60.0)),
        (Side::N, lot.rand(-60.0, 60.0)),
    ];
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Tiles);
    lot.loot(inside.center() - v(60.0, 0.0), 3);
    lot.loot(inside.center() + v(60.0, 0.0), 3);
    let r = PropKind::Safe.radius() + 4.0;
    let p = lot.corner(inside, r);
    lot.prop(p, PropKind::Safe, 3);
    rubble(lot, 4);
    yard(lot, 3, 2, 1);
}

/// A gas station: a shop, two pump islands, and far too many fuel barrels.
pub(super) fn gas_station(lot: &mut Lot) {
    let tier = lot.tier;
    let mut doors = vec![(Side::S, lot.rand(-50.0, 50.0))];
    if lot.chance(0.5) {
        doors.push((Side::E, 0.0));
    }
    let shop = lot.building(
        v(-HALF.x + 150.0, HALF.y - 100.0),
        v(300.0, 200.0),
        &doors,
        Roof::Tin,
        Floor::Tiles,
    );
    lot.loot(shop.center() - v(50.0, 0.0), tier);
    lot.loot(shop.center() + v(50.0, 0.0), tier);
    let base_x = lot.rand(-20.0, 40.0);
    for y in [-40.0, -220.0] {
        for dx in [0.0, 160.0] {
            lot.wall(v(base_x + dx, y), v(36.0, 60.0), WallStyle::Metal);
        }
    }
    wreck(lot);
    yard(lot, 2, 4, 1);
}

/// A hospital: lobby in the middle, wards on each side, medical caches everywhere.
pub(super) fn hospital(lot: &mut Lot) {
    let tier = lot.tier;
    let size = v(lot.rand(600.0, 680.0), lot.rand(420.0, 460.0));
    let c = v(0.0, 30.0);
    let h = size / 2.0;
    let doors = [
        (Side::S, 0.0),
        (Side::W, lot.rand(-60.0, 60.0)),
        (Side::E, lot.rand(-60.0, 60.0)),
    ];
    let inside = lot.building(c, size, &doors, Roof::Concrete, Floor::Tiles);
    let wing = h.x * 0.38;
    let left_gap = c.y + lot.rand(-80.0, 80.0);
    let right_gap = c.y + lot.rand(-80.0, 80.0);
    lot.run_v(inside.min.y, inside.max.y, c.x - wing, &[left_gap]);
    lot.run_v(inside.min.y, inside.max.y, c.x + wing, &[right_gap]);
    lot.loot(v(c.x, c.y + 60.0), tier);
    for sx in [-1.0, 1.0] {
        let x = c.x + sx * (wing + inside.max.x - c.x) / 2.0;
        lot.loot(v(x, c.y + h.y / 2.0), tier);
        lot.loot(v(x, c.y - h.y / 2.0), 0);
    }
    lot.loot(v(c.x, c.y - 80.0), 0);
    wreck(lot);
    yard(lot, 2, 1, 0);
}

/// A military checkpoint: sandbag lines, a guard post, and crates of army surplus.
pub(super) fn checkpoint(lot: &mut Lot) {
    let next = lot.next_tier();
    let side = [Side::N, Side::S, Side::E, Side::W][lot.b.rng.below(4)];
    let post = v(lot.rand(-200.0, 200.0), lot.rand(-250.0, 250.0));
    let inside = lot.building(
        post,
        v(180.0, 180.0),
        &[(side, 0.0)],
        Roof::Concrete,
        Floor::Concrete,
    );
    lot.loot(inside.center(), next);
    let area = lot.area();
    for _ in 0..6 {
        let size = if lot.chance(0.5) {
            v(lot.rand(120.0, 200.0), 26.0)
        } else {
            v(26.0, lot.rand(120.0, 200.0))
        };
        lot.scatter_solid(area, size, WallStyle::Sandbag);
    }
    for _ in 0..4 {
        lot.scatter_prop(area, PropKind::Crate, next, true);
    }
    let tier = lot.tier;
    lot.scatter_loot(area, next);
    lot.scatter_loot(area, tier);
    yard(lot, 0, 1, 0);
}

/// A supermarket: long aisles of shelving to fight through.
pub(super) fn supermarket(lot: &mut Lot) {
    let tier = lot.tier;
    let c = v(0.0, lot.rand(0.0, 40.0));
    let size = v(620.0, 400.0);
    let doors = [
        (Side::S, -180.0),
        (Side::S, 180.0),
        (Side::N, lot.rand(-150.0, 150.0)),
    ];
    lot.building(c, size, &doors, Roof::Tin, Floor::Tiles);
    for x in [-180.0, -60.0, 60.0, 180.0] {
        lot.wall(v(c.x + x, c.y), v(20.0, 160.0), WallStyle::Metal);
    }
    for x in [-120.0, 0.0, 120.0] {
        let y = c.y + lot.rand(-60.0, 60.0);
        lot.loot(v(c.x + x, y), tier);
    }
    wreck(lot);
    wreck(lot);
    yard(lot, 2, 2, 1);
}
