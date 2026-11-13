//! Regression coverage for carrying akimbo intent through the saved-recipe lifecycle.
use super::*;
use crate::weapon::akimbo::Intent;

fn paired() -> WeaponRecipe {
    let mut recipe = WeaponRecipe::new_weapon("parhelion.akimbo-lifecycle").unwrap();
    recipe.overrides.akimbo = Some(Intent::default());
    recipe
}

#[test]
fn akimbo_recipe_round_trip_reaches_the_compiler_and_changes_the_fingerprint() {
    let paired = paired();
    let mut single = paired.clone();
    single.overrides.akimbo = None;
    let single_json = single.to_json_pretty().unwrap();
    assert!(!single_json.contains("\"akimbo\""));
    assert_eq!(WeaponRecipe::from_json_str(&single_json).unwrap(), single);

    let encoded = paired.to_json_pretty().unwrap();
    let decoded = WeaponRecipe::from_json_str(&encoded).unwrap();
    assert_eq!(decoded, paired);
    assert_eq!(
        decoded.to_spec().unwrap().overrides.akimbo,
        paired.overrides.akimbo
    );
    assert!(!single.same_saved_content(&paired));
    assert_ne!(
        crate::manifest::recipe_selection_fingerprint(&[single]).unwrap(),
        crate::manifest::recipe_selection_fingerprint(&[paired]).unwrap(),
    );
}

#[test]
fn akimbo_intent_survives_base_and_appearance_changes() {
    let mut recipe = paired();
    recipe.overrides.akimbo.as_mut().unwrap().gap_mm = 240;
    let intent = recipe.overrides.akimbo.clone();
    recipe.set_donor(MOUNTAINTOP_DONOR_HASH, MOUNTAINTOP_DONOR_NAME);
    assert_eq!(recipe.overrides.akimbo, intent);
    recipe.set_presentation_donor(Some(WeaponDonorReference {
        item_hash: ARC_LOGIC_DONOR_HASH.into(),
        expected_name: Some(ARC_LOGIC_DONOR_NAME.into()),
    }));
    assert_eq!(recipe.overrides.akimbo, intent);
    assert_eq!(recipe.to_spec().unwrap().overrides.akimbo, intent);
}

#[test]
fn akimbo_intent_survives_duplicate_rename_and_authored_base_rebasing() {
    let mut base = paired();
    base.overrides.akimbo.as_mut().unwrap().gap_mm = 520;
    let mut copy = base.unused_copy([base.namespace.as_str()]).unwrap();
    assert_ne!(copy.identity, base.identity);
    assert_eq!(copy.overrides.akimbo, base.overrides.akimbo);
    copy.rename_authored_item("Renamed Akimbo").unwrap();
    assert_eq!(copy.overrides.akimbo, base.overrides.akimbo);

    let own = WeaponRecipe::new_weapon("parhelion.akimbo-derived").unwrap();
    let rebased = own.rebased_onto(&base).unwrap();
    assert_eq!(rebased.identity, own.identity);
    assert_eq!(rebased.overrides.akimbo, base.overrides.akimbo);
}

#[test]
fn akimbo_gap_is_preserved_in_compilation_and_invalidates_the_recipe_fingerprint() {
    let original = paired();
    let mut adjusted = original.clone();
    adjusted.overrides.akimbo.as_mut().unwrap().gap_mm = 240;
    let decoded = WeaponRecipe::from_json_str(&adjusted.to_json_pretty().unwrap()).unwrap();
    assert_eq!(
        decoded.to_spec().unwrap().overrides.akimbo.unwrap().gap_mm,
        240
    );
    assert!(!adjusted.same_saved_content(&original));
    assert_ne!(
        crate::manifest::recipe_selection_fingerprint(&[original]).unwrap(),
        crate::manifest::recipe_selection_fingerprint(&[adjusted]).unwrap(),
    );
}

#[test]
fn akimbo_reload_policy_is_preserved_and_changes_the_recipe_fingerprint() {
    let lower = paired();
    let mut non_full = lower.clone();
    non_full.overrides.akimbo.as_mut().unwrap().reload =
        crate::weapon::akimbo::Reload::NonFullHands;
    let mut empty_only = lower.clone();
    empty_only.overrides.akimbo.as_mut().unwrap().reload =
        crate::weapon::akimbo::Reload::EmptyHands;
    let mut fingerprints = std::collections::HashSet::new();
    for recipe in [&lower, &non_full, &empty_only] {
        let decoded = WeaponRecipe::from_json_str(&recipe.to_json_pretty().unwrap()).unwrap();
        assert_eq!(
            decoded.to_spec().unwrap().overrides.akimbo,
            recipe.overrides.akimbo
        );
        assert!(fingerprints.insert(
            crate::manifest::recipe_selection_fingerprint(std::slice::from_ref(recipe)).unwrap()
        ));
        assert_eq!(
            recipe
                .unused_copy([recipe.namespace.as_str()])
                .unwrap()
                .overrides
                .akimbo,
            recipe.overrides.akimbo
        );
    }
    assert!(!non_full.same_saved_content(&empty_only));
    assert!(!lower.same_saved_content(&empty_only));
    assert!(!lower.same_saved_content(&non_full));
    assert_ne!(
        crate::manifest::recipe_selection_fingerprint(&[non_full]).unwrap(),
        crate::manifest::recipe_selection_fingerprint(&[empty_only]).unwrap(),
    );
}

#[test]
fn akimbo_is_rejected_for_every_nonweapon_kind() {
    for kind in ItemKind::ALL.into_iter().filter(|kind| !kind.is_weapon()) {
        let mut recipe = paired();
        recipe.kind = kind;
        assert!(
            recipe
                .validate()
                .unwrap_err()
                .to_string()
                .contains("Only weapons can be akimbo"),
            "{kind:?}",
        );
    }
}

#[test]
fn akimbo_saved_recipes_reject_unknown_capabilities_versions_and_mixed_pairs() {
    let encoded = serde_json::to_value(paired()).unwrap();
    for bad in [
        serde_json::json!({"schema": 1, "mode": "independent", "reload": "empty_hands", "runtime_compatible": true}),
        serde_json::json!({"schema": 2, "mode": "independent", "reload": "empty_hands"}),
        serde_json::json!({"schema": 1, "mode": "independent", "reload": "empty_hands", "offhand": {"namespace": "parhelion.offhand", "content_sha256": "A".repeat(64)}}),
    ] {
        let mut invalid = encoded.clone();
        invalid["overrides"]["akimbo"] = bad;
        assert!(WeaponRecipe::from_json_str(&invalid.to_string()).is_err());
    }
}

#[test]
fn akimbo_snapshot_refuses_before_creating_stage_or_reading_packages() {
    let directory = tempfile::tempdir().unwrap();
    let packages = directory.path().join("missing-packages");
    let staging = directory.path().join("staging");
    let result = crate::workflow::BatchBuildSnapshot::new(crate::workflow::BatchBuildRequest {
        package_directory: packages.clone(),
        staging_root: staging.clone(),
        ignore_installed_authored_overlays: true,
        recipes: vec![paired()],
        maps: Vec::new(),
    });
    let error = result.unwrap_err();
    assert!(
        error.contains("Akimbo requires a compatible native runtime"),
        "{error}"
    );
    assert!(!staging.exists());
    assert!(!packages.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn akimbo_added_after_snapshot_cannot_skip_the_native_capability_check() {
    let directory = tempfile::tempdir().unwrap();
    let mut recipe = paired();
    recipe.overrides.akimbo = None;
    let mut snapshot =
        crate::workflow::BatchBuildSnapshot::new(crate::workflow::BatchBuildRequest {
            package_directory: directory.path().join("missing-packages"),
            staging_root: directory.path().join("staging"),
            ignore_installed_authored_overlays: true,
            recipes: vec![recipe],
            maps: Vec::new(),
        })
        .unwrap();
    snapshot.request.recipes[0].overrides.akimbo = Some(Intent::default());
    let failure =
        crate::workflow::build_and_stage_snapshot_reporting(&snapshot, |_| {}).unwrap_err();
    assert!(
        failure.to_string().contains("gained akimbo intent"),
        "{failure}"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
