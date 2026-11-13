//! All compiler entry points must reject malformed akimbo intent before opening source tables.
use super::*;

fn invalid_paired_spec() -> WeaponCloneSpec {
    let mut spec = crate::recipe::WeaponRecipe::new_weapon("parhelion.akimbo-build-gate")
        .unwrap()
        .to_spec()
        .unwrap();
    spec.overrides.akimbo = Some(crate::weapon::akimbo::Intent {
        schema: crate::weapon::akimbo::INTENT_SCHEMA + 1,
        ..Default::default()
    });
    spec
}

#[test]
fn akimbo_direct_canonical_compile_cannot_bypass_intent_validation() {
    let directory = tempfile::tempdir().unwrap();
    let packages = directory.path().join("missing-packages");
    let result =
        super::super::build::build_weapon_project_canonical(&packages, &[invalid_paired_spec()]);
    let error = result.unwrap_err().to_string();
    assert!(error.contains("Unsupported akimbo schema"), "{error}");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn akimbo_catalog_validated_compile_still_validates_intent() {
    let directory = tempfile::tempdir().unwrap();
    let packages = directory.path().join("missing-packages");
    let result = crate::item::build_weapon_project_after_catalog_validation(
        &packages,
        &WeaponProjectSpec {
            weapons: vec![invalid_paired_spec()],
        },
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("Unsupported akimbo schema"), "{error}");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
