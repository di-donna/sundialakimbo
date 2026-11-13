use super::*;
use crate::weapon::akimbo::deployment;

fn descriptor_bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&deployment::test_descriptor()).unwrap()
}

fn legacy_descriptor_bytes() -> Vec<u8> {
    historical_descriptor_bytes(1)
}

fn historical_descriptor_bytes(schema: u32) -> Vec<u8> {
    let mut descriptor = serde_json::to_value(deployment::test_descriptor()).unwrap();
    descriptor["schema_version"] = schema.into();
    let profile = descriptor["profiles"][0].as_object_mut().unwrap();
    if schema < 3 {
        profile.remove("reload_policy");
    } else {
        profile.insert("reload_policy".into(), 2.into());
    }
    if schema == 1 {
        profile.remove("gap_mm");
    }
    let mut bytes = serde_json::to_vec_pretty(&descriptor).unwrap();
    bytes.extend_from_slice(b"\r\n  ");
    bytes
}

#[test]
fn akimbo_descriptor_retires_with_packages_and_rolls_back_after_interruption() {
    for bytes in [
        descriptor_bytes(),
        legacy_descriptor_bytes(),
        historical_descriptor_bytes(2),
        historical_descriptor_bytes(3),
    ] {
        let fixture = Fixture::new();
        let path = fs::canonicalize(&fixture.target)
            .unwrap()
            .join(deployment::FILE_NAME);
        fs::write(&path, &bytes).unwrap();
        let result = install_staged_packages_inner(
            &fixture.request(),
            Some(AUTHORED_PACKAGES.len() + 1),
            DEFAULT_CACHE_INVALIDATION_OPS,
        );
        let error = result.unwrap_err();
        assert!(
            error
                .rollback
                .as_ref()
                .is_some_and(|rollback| rollback.succeeded()),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let report = install_staged_packages(&fixture.request()).unwrap();
        assert!(report.removed_obsolete_packages.contains(&path));
        assert!(!path.exists());
        assert_eq!(
            fs::read(report.backup_directory.join(deployment::FILE_NAME)).unwrap(),
            bytes
        );
    }
}

#[test]
fn akimbo_descriptor_recovery_restores_previous_generation_and_keeps_external_edits() {
    for original in [
        descriptor_bytes(),
        legacy_descriptor_bytes(),
        historical_descriptor_bytes(2),
        historical_descriptor_bytes(3),
    ] {
        for concurrent in [false, true] {
            let fixture = Fixture::new();
            let (mut record, _) =
                fixture.write_synthetic_transaction(InstallTransactionState::Pending);
            let mut next = deployment::test_descriptor();
            next.generation = "2".repeat(32);
            let authored = serde_json::to_vec_pretty(&next).unwrap();
            let path = fixture.target.join(deployment::FILE_NAME);
            fs::write(
                record.backup_directory.join(deployment::FILE_NAME),
                &original,
            )
            .unwrap();
            fs::write(
                &path,
                if concurrent {
                    b"external change".as_slice()
                } else {
                    &authored
                },
            )
            .unwrap();
            record.artifacts.push(InstallTransactionArtifact {
                remove_target: false,
                file_name: deployment::FILE_NAME.into(),
                authored: TransactionDigest::from(&crate::artifact::digest_bytes(&authored)),
                original: Some(TransactionDigest::from(&crate::artifact::digest_bytes(
                    &original,
                ))),
                backup_file_name: Some(deployment::FILE_NAME.into()),
                temporary_file_name: ".parhelion-install-akimbo-test.tmp".into(),
            });
            write_install_transaction(&fixture.target.join(INSTALL_TRANSACTION_FILE_NAME), &record)
                .unwrap();
            let result = recover_interrupted_install(&fixture.recovery_request());
            if concurrent {
                assert!(result.is_err());
                assert_eq!(fs::read(&path).unwrap(), b"external change");
            } else {
                assert!(matches!(result.unwrap(), RecoveryOutcome::Recovered { .. }));
                assert_eq!(fs::read(&path).unwrap(), original);
            }
        }
    }
}

#[test]
fn akimbo_descriptor_is_removed_and_backed_up_by_normal_uninstall() {
    for bytes in [
        descriptor_bytes(),
        legacy_descriptor_bytes(),
        historical_descriptor_bytes(2),
        historical_descriptor_bytes(3),
    ] {
        let fixture = installed_fixture();
        let path = fs::canonicalize(&fixture.target)
            .unwrap()
            .join(deployment::FILE_NAME);
        fs::write(&path, &bytes).unwrap();
        let plan = preview_uninstall(&fixture.target).unwrap();
        assert!(
            plan.artifacts()
                .iter()
                .any(|artifact| artifact.file_name == deployment::FILE_NAME)
        );
        let report = uninstall_fixture(&plan, &fixture.backups, game_stopped).unwrap();
        assert!(report.removed_files.contains(&path));
        assert!(!path.exists());
        assert_eq!(
            fs::read(report.backup_directory.join(deployment::FILE_NAME)).unwrap(),
            bytes
        );
    }
}

#[test]
fn akimbo_missing_binding_and_unlisted_binding_refuse_before_mutation() {
    let fixture = Fixture::new();
    assert!(
        crate::install::akimbo::validate(&fixture.staging, &fixture.target, None, &[], &[], 1)
            .is_err()
    );
    fs::write(
        fixture.staging.join(deployment::FILE_NAME),
        descriptor_bytes(),
    )
    .unwrap();
    assert!(
        install_staged_packages(&fixture.request())
            .unwrap_err()
            .message
            .contains("Unlisted akimbo descriptor")
    );
    assert!(!fixture.backups.exists());
    assert!(!fixture.target.join(deployment::FILE_NAME).exists());
}

fn stage_binding(fixture: &Fixture) -> ArtifactMetadata {
    let runtime_directory = fixture.target.parent().unwrap().join("bin/x64");
    let runtime = runtime_directory.join("steam_api64.dll");
    fs::write(
        &runtime,
        sundial::test_support::runtime::module("Dawn", [1, 0, 0, 0], 0),
    )
    .unwrap();
    let capability = deployment::Capability {
        schema_version: deployment::SCHEMA,
        game_build: deployment::GAME_BUILD,
        runtime_sha256: digest_file(&runtime).unwrap().sha256,
        adapter_ids: vec![7],
        max_profiles: deployment::MAX_PROFILES as u32,
        descriptor_schema: deployment::SCHEMA,
    };
    let capability_path = runtime_directory.join(deployment::CAPABILITY_PATH);
    fs::create_dir_all(capability_path.parent().unwrap()).unwrap();
    fs::write(&capability_path, serde_json::to_vec(&capability).unwrap()).unwrap();
    let installed = deployment::InstalledCapability::read(&fixture.target).unwrap();
    let sources: Vec<ArtifactMetadata> =
        serde_json::from_value(serde_json::Value::Array(fixture.source_artifact_json())).unwrap();
    let artifacts: Vec<ArtifactMetadata> =
        serde_json::from_value(serde_json::Value::Array(fixture.artifact_json())).unwrap();
    // These native references and transitive reads are outside the canonical table inputs.
    for (id, patch) in [(0x0696, 0), (0x0600, 0), (0x0600, 1)] {
        fs::write(
            fixture
                .target
                .join(format!("w64_akimbo_test_{id:04x}_{patch}.pkg")),
            package_bytes(id, patch, 0xAABB_CCDD_EEFF_0011, b"native source"),
        )
        .unwrap();
    }
    deployment::stage(
        &fixture.target,
        &fixture.staging,
        &sources,
        &artifacts,
        &deployment::test_descriptor().profiles,
        &[tiger_pkg::TagHash::new(0x0600, 1).0],
        Some(&installed),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn akimbo_staged_binding_validates_real_files_and_rejects_changed_host_before_install() {
    let fixture = Fixture::new();
    let metadata = stage_binding(&fixture);
    let sources: Vec<ArtifactMetadata> =
        serde_json::from_value(serde_json::Value::Array(fixture.source_artifact_json())).unwrap();
    let artifacts: Vec<ArtifactMetadata> =
        serde_json::from_value(serde_json::Value::Array(fixture.artifact_json())).unwrap();
    let bindings = crate::install::akimbo::validate(
        &fixture.staging,
        &fixture.target,
        Some(metadata),
        &sources,
        &artifacts,
        1,
    )
    .unwrap()
    .unwrap();
    bindings.verify(&fixture.staging, &fixture.target).unwrap();
    let extra_patch = fixture.target.join("w64_akimbo_test_0600_2.pkg");
    fs::write(
        &extra_patch,
        package_bytes(0x0600, 2, 0xAABB_CCDD_EEFF_0011, b"new patch"),
    )
    .unwrap();
    assert!(
        bindings
            .verify(&fixture.staging, &fixture.target)
            .unwrap_err()
            .contains("Unpinned package")
    );
    fs::remove_file(extra_patch).unwrap();
    fs::write(
        fixture
            .target
            .parent()
            .unwrap()
            .join("bin/x64/steam_api64.dll"),
        b"changed runtime",
    )
    .unwrap();
    assert!(bindings.verify(&fixture.staging, &fixture.target).is_err());
    assert!(!fixture.backups.exists());
    assert!(!fixture.target.join(deployment::FILE_NAME).exists());
    assert!(!fixture.target.join(INSTALL_TRANSACTION_FILE_NAME).exists());
}

fn staged_paired_generation() -> Fixture {
    let fixture = Fixture::new();
    let metadata = stage_binding(&fixture);
    let mut recipe = WeaponRecipe::new_weapon("parhelion.akimbo-upgrade").unwrap();
    recipe.overrides.akimbo = Some(Default::default());
    let spec = recipe.to_spec().unwrap();
    let mut manifest = fixture.manifest_json(None);
    let mut project = serde_json::to_value(test_manifest_project_with_unlocks(&[(
        300,
        ACCOUNT_UNLOCK_BANK,
        1,
    )]))
    .unwrap();
    let weapon = &mut project["weapons"][0];
    weapon["namespace"] = recipe.namespace.clone().into();
    weapon["name"] = recipe.name.clone().into();
    weapon["item"]["hash"] = format!("0x{:08X}", spec.identity.item_hash).into();
    weapon["collectible"]["hash"] = format!("0x{:08X}", spec.identity.collectible_hash).into();
    weapon["unlock"]["hash"] = format!("0x{:08X}", spec.identity.unlock_hash).into();
    weapon["donor"]["item_hash"] = format!("0x{:08X}", spec.donor_item_hash).into();
    weapon["item"]["definition_tag"] = "0x81D40010".into();
    weapon["item"]["string_tag"] = "0x81D40011".into();
    weapon["donor"]["definition_tag"] = "0x81D40012".into();
    weapon["donor"]["string_tag"] = "0x81D40013".into();
    project["sunrise"]["watermarked_icon_containers"] = json!(["0x81D40002"]);
    manifest["project"] = project;
    manifest["akimbo_bindings"] = serde_json::to_value(metadata).unwrap();
    manifest["selected_recipe_files"] = json!(["recipes/akimbo-upgrade.parhelion.json"]);
    manifest["selection_fingerprint"] = recipe_selection_fingerprint(std::slice::from_ref(&recipe))
        .unwrap()
        .into();
    serde_json::from_value::<ManifestDocument>(manifest.clone())
        .unwrap()
        .validate()
        .unwrap();
    fs::create_dir(fixture.staging.join("recipes")).unwrap();
    fs::write(
        fixture
            .staging
            .join("recipes/akimbo-upgrade.parhelion.json"),
        crate::manifest::canonical_recipe(&recipe).unwrap(),
    )
    .unwrap();
    fs::write(
        fixture.staging.join(MANIFEST_FILE_NAME),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fixture
}

#[test]
fn akimbo_historical_upgrade_backs_up_exact_bytes_and_restores_after_interruption() {
    for schema in [1, 2, 3] {
        let fixture = staged_paired_generation();
        let old = historical_descriptor_bytes(schema);
        let path = fixture.target.join(deployment::FILE_NAME);
        fs::write(&path, &old).unwrap();
        let mut request = fixture.request();
        request.runtime_snapshot_check = test_dawn_runtime_snapshot;
        let failed = install_staged_packages_inner(
            &request,
            Some(AUTHORED_PACKAGES.len() + 1),
            DEFAULT_CACHE_INVALIDATION_OPS,
        )
        .unwrap_err();
        assert!(
            failed
                .rollback
                .as_ref()
                .is_some_and(|result| result.succeeded()),
            "{failed}"
        );
        assert_eq!(fs::read(&path).unwrap(), old);
        let report = install_staged_packages(&request).unwrap();
        assert_eq!(
            fs::read(report.backup_directory.join(deployment::FILE_NAME)).unwrap(),
            old
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            fs::read(fixture.staging.join(deployment::FILE_NAME)).unwrap()
        );
        assert_eq!(
            deployment::Descriptor::read(&path).unwrap().schema_version,
            deployment::SCHEMA
        );
        assert!(!fixture.target.join(INSTALL_TRANSACTION_FILE_NAME).exists());
    }
}

#[test]
fn akimbo_malformed_historical_descriptor_is_preserved_before_removal_or_upgrade() {
    for schema in [1, 2, 3] {
        for upgrade in [false, true] {
            let fixture = if upgrade {
                staged_paired_generation()
            } else {
                Fixture::new()
            };
            let mut malformed: serde_json::Value =
                serde_json::from_slice(&historical_descriptor_bytes(schema)).unwrap();
            malformed["profiles"][0][if schema == 1 {
                "gap_mm"
            } else {
                "reload_policy"
            }] = 360.into();
            let malformed = serde_json::to_vec(&malformed).unwrap();
            let path = fixture.target.join(deployment::FILE_NAME);
            fs::write(&path, &malformed).unwrap();
            let mut request = fixture.request();
            if upgrade {
                request.runtime_snapshot_check = test_dawn_runtime_snapshot;
            }
            let error = install_staged_packages(&request).unwrap_err();
            assert!(error.message.contains("akimbo descriptor"), "{error}");
            assert!(preview_uninstall(&fixture.target).is_err());
            assert_eq!(fs::read(&path).unwrap(), malformed);
            assert!(!fixture.backups.exists());
            assert!(!fixture.target.join(INSTALL_TRANSACTION_FILE_NAME).exists());
            for package in AUTHORED_PACKAGES {
                assert!(!fixture.target.join(package.file_name).exists());
            }
        }
    }
}

#[test]
fn akimbo_historical_schema_is_never_accepted_as_a_new_staged_generation() {
    for schema in [1, 2, 3] {
        let fixture = staged_paired_generation();
        let bytes = historical_descriptor_bytes(schema);
        fs::write(fixture.staging.join(deployment::FILE_NAME), &bytes).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.staging.join(MANIFEST_FILE_NAME)).unwrap())
                .unwrap();
        let digest = crate::artifact::digest_bytes(&bytes);
        manifest["akimbo_bindings"]["byte_length"] = digest.byte_length.into();
        manifest["akimbo_bindings"]["sha256"] = digest.sha256.into();
        fs::write(
            fixture.staging.join(MANIFEST_FILE_NAME),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut request = fixture.request();
        request.runtime_snapshot_check = test_dawn_runtime_snapshot;
        let error = install_staged_packages(&request).unwrap_err();
        assert!(error.message.contains("akimbo descriptor"), "{error}");
        assert!(!fixture.target.join(deployment::FILE_NAME).exists());
        assert!(!fixture.backups.exists());
    }
}
