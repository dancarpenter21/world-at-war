use super::*;

#[test]
fn role_summary_marks_only_the_requesting_players_held_role_without_exposing_owner_identity() {
    let mut game = authority_tests::game();
    let role = game.roles.get_mut(&Uuid::from_u128(106)).unwrap();
    let owner = Uuid::from_u128(8_001);
    role.owner = Some(owner);
    assert!(role_summary(role, Some(owner)).held_by_you);
    for player in [None, Some(Uuid::from_u128(8_099))] {
        let summary = role_summary(role, player);
        assert!(summary.held);
        assert!(!summary.held_by_you);
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains(&owner.to_string()));
        assert!(!json.contains("owner"));
    }
}

#[test]
fn vacant_roles_are_never_marked_as_the_callers_held_slot() {
    let game = authority_tests::game();
    let role = game.roles.get(&Uuid::from_u128(106)).unwrap();
    for player in [None, Some(Uuid::from_u128(8_001))] {
        let summary = role_summary(role, player);
        assert!(!summary.held);
        assert!(!summary.held_by_you);
    }
}
