use evoclash_game::evolution::Perk;

use super::*;

#[test]
fn evolution_resets_when_a_match_starts() {
    let mut app = App::new();
    app.init_resource::<LootState>()
        .init_resource::<PropSlots>()
        .init_resource::<Blasts>()
        .insert_resource(ResetRequested(true));
    app.add_systems(Update, reset_match);
    let mut evo = Evolution::default();
    evo.add_xp(500);
    evo.ranks[Perk::Claws.index()] = 2;
    let p = app
        .world_mut()
        .spawn((
            Player {
                client_id: 1,
                name: "p".into(),
                color: [1.0; 3],
                bot: false,
            },
            Vitals::default(),
            Loadout::default(),
            Stats {
                level: evo.level,
                ..default()
            },
            evo,
            SkillState {
                cd_a: 50,
                ..default()
            },
            Evolving::default(),
        ))
        .id();
    app.update();
    let world = app.world();
    assert_eq!(world.get::<Evolution>(p).unwrap(), &Evolution::default());
    assert_eq!(world.get::<SkillState>(p).unwrap(), &SkillState::default());
    assert_eq!(world.get::<Stats>(p).unwrap().level, 1);
}

#[test]
fn catch_up_kit_grows_with_the_gap() {
    let mut rng = rand::rng();
    assert_eq!(catch_up_kit(0, &mut rng), (None, None));
    assert_eq!(catch_up_kit(2, &mut rng), (None, None));
    assert!(matches!(
        catch_up_kit(4, &mut rng),
        (Some(WeaponKind::Bow | WeaponKind::PipeShotgun), None)
    ));
    assert_eq!(
        catch_up_kit(8, &mut rng),
        (Some(WeaponKind::Rifle), Some(ArmorKind::Scrap))
    );
    assert_eq!(
        catch_up_kit(12, &mut rng),
        (Some(WeaponKind::Rifle), Some(ArmorKind::Kevlar))
    );
}
