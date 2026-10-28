use super::*;

/// Each match has a new layout, so the grid is rebuilt whenever the arena changes.
pub(super) fn rebuild_nav(mut commands: Commands, arena: Res<Arena>, nav: Option<Res<Nav>>) {
    if nav.is_none() || arena.is_changed() {
        commands.insert_resource(Nav(NavGrid::new(&arena, PLAYER_RADIUS + 2.0)));
    }
}
