use super::*;

#[test]
fn akimbo_finite_core_domains_exclude_inherited_stock_overlay_entries() {
    for (package, entry, expected) in [
        (0x01bb, 6467, false),
        (0x01bb, 6468, true),
        (0x01bb, 8191, true),
        (0x0a9f, 8191, false),
        (0x0aa0, 0, true),
        (0x0cff, 8191, true),
        (0x0d00, 0, false),
    ] {
        let tag = tiger_pkg::TagHash::new(package, entry).0;
        assert_eq!(private_core_tag(tag), expected, "{tag:08X}");
    }
    for tag in [0, 0x8000_0000, u32::MAX, 0xa1d4_0000] {
        assert!(!private_core_tag(tag));
    }
}

#[test]
fn akimbo_finite_burst_requires_its_own_host_capability() {
    let mut descriptor = test_descriptor();
    let profile = &mut descriptor.profiles[0];
    profile.adapter = 11;
    profile.rig = Rig {
        animation_tag: 0x81d40010,
        skeleton_tag: 0x81d40011,
        rig_tag: 0x81d40012,
    };
    profile.pose = Some(GunPose {
        weapon_bone_count: 6,
        marker_bone: 1,
        marker_bind_bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
        control_count: 76,
        animation_bytes: 0x3500,
    });
    profile.validate().unwrap();
    let encoded = serde_json::to_vec(&descriptor).unwrap();
    let decoded: Descriptor = serde_json::from_slice(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.profiles[0].adapter, 11);
    let mut installed = InstalledCapability {
        capability: Capability {
            schema_version: SCHEMA,
            game_build: GAME_BUILD,
            runtime_sha256: descriptor.runtime_sha256.clone(),
            adapter_ids: vec![7, 10],
            max_profiles: MAX_PROFILES as u32,
            descriptor_schema: SCHEMA,
        },
        runtime_sha256: descriptor.runtime_sha256.clone(),
        exe_sha256: descriptor.exe_sha256.clone(),
    };
    assert!(
        installed
            .require_profiles(&descriptor.profiles)
            .unwrap_err()
            .contains("adapter 11")
    );
    installed.capability.adapter_ids.push(11);
    installed.require_profiles(&descriptor.profiles).unwrap();
    let artifact = ArtifactMetadata {
        file_name: crate::package_profile::PARHELION_ASSET_FILE_NAME.into(),
        byte_length: 1,
        sha256: "A".repeat(64),
    };
    // Startup interception must recognize only a privately emitted core. A
    // hash-pinned stock dependency alone cannot authorize that ownership gate.
    descriptor.source_artifacts.push(Package {
        path: "packages/stock_core.pkg".into(),
        sha256: "B".repeat(64),
    });
    assert!(
        descriptor
            .verify_package_metadata(&[], std::slice::from_ref(&artifact))
            .unwrap_err()
            .contains("core is outside")
    );
    descriptor.profiles[0].adapter = 10;
    descriptor
        .verify_package_metadata(&[], std::slice::from_ref(&artifact))
        .unwrap();
    descriptor.profiles[0].adapter = 11;
    descriptor.profiles[0].barrel.core_tag = descriptor.profiles[0].model_tag + 3;
    descriptor
        .verify_package_metadata(&[], std::slice::from_ref(&artifact))
        .unwrap();
    let runtime = ArtifactMetadata {
        file_name: "w64_sandbox_01bb_7.pkg".into(),
        byte_length: 1,
        sha256: "C".repeat(64),
    };
    descriptor.packages.push(Package {
        path: format!("packages/{}", runtime.file_name),
        sha256: runtime.sha256.clone(),
    });
    let artifacts = [artifact, runtime];
    descriptor.profiles[0].barrel.core_tag = tiger_pkg::TagHash::new(0x01bb, 6467).0;
    assert!(descriptor.verify_package_metadata(&[], &artifacts).is_err());
    descriptor.profiles[0].barrel.core_tag = tiger_pkg::TagHash::new(0x01bb, 6468).0;
    descriptor.verify_package_metadata(&[], &artifacts).unwrap();
    let mut bad = decoded.profiles[0].clone();
    bad.pose = None;
    assert!(bad.validate().is_err());
    let mut bad = decoded.profiles[0].clone();
    bad.visual = None;
    assert!(bad.validate().is_err());
    let mut bad = decoded.profiles[0].clone();
    bad.adapter = 12;
    assert!(bad.validate().is_err());
}

#[test]
fn akimbo_default_content_variant_has_explicit_bounded_sentinel() {
    let mut descriptor = test_descriptor();
    let profile = &mut descriptor.profiles[0];
    profile.adapter = 10;
    profile.rig = Rig {
        animation_tag: 0x81D40010,
        skeleton_tag: 0x81D40011,
        rig_tag: 0x81D40012,
    };
    profile.pose = Some(GunPose {
        weapon_bone_count: 6,
        marker_bone: 1,
        marker_bind_bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
        control_count: 76,
        animation_bytes: 0x3500,
    });
    descriptor.profiles[0].barrel.variant = u32::MAX;
    descriptor.validate().unwrap();
    descriptor.profiles[0].barrel.property_count = 0;
    descriptor.validate().unwrap();
    descriptor.profiles[0].barrel.variant = 0;
    assert!(descriptor.validate().is_err());
    descriptor.profiles[0].barrel.variant = u32::MAX;
    let definition = descriptor.profiles[0].barrel.content_definition;
    descriptor.profiles[0].barrel.content_size = definition + 0x270;
    descriptor.validate().unwrap();
    descriptor.profiles[0].barrel.content_size -= 1;
    assert!(descriptor.validate().is_err());
    descriptor.profiles[0].barrel.content_size = definition + 0x270;
    descriptor.profiles[0].barrel.property_count = 1;
    assert!(descriptor.validate().is_err());
}

#[test]
fn akimbo_attachment_bones_require_unique_roles_and_real_private_rig_bones() {
    let mut descriptor = test_descriptor();
    let profile = &mut descriptor.profiles[0];
    profile.adapter = 10;
    profile.rig = Rig {
        animation_tag: 0x81D40010,
        skeleton_tag: 0x81D40011,
        rig_tag: 0x81D40012,
    };
    let identity = [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()];
    profile.pose = Some(GunPose {
        weapon_bone_count: 6,
        marker_bone: 1,
        marker_bind_bits: identity,
        control_count: 76,
        animation_bytes: 0x3500,
    });
    profile.visual.as_mut().unwrap().marker_bones = vec![MarkerBone {
        role: 1,
        index: 0,
        bone: 2,
        bind_bits: identity,
    }];
    descriptor.validate().unwrap();
    let mut scaled = descriptor.clone();
    scaled.profiles[0].visual.as_mut().unwrap().muzzle_bits[7] = 1.4256907f32.to_bits();
    scaled.validate().unwrap();
    let mut legacy_scaled = test_descriptor();
    legacy_scaled.profiles[0]
        .visual
        .as_mut()
        .unwrap()
        .muzzle_bits[7] = 1.4256907f32.to_bits();
    assert!(legacy_scaled.validate().is_err());
    for scale in [0.0f32, 0.5, 2.0, f32::INFINITY, f32::NAN] {
        scaled.profiles[0].visual.as_mut().unwrap().muzzle_bits[7] = scale.to_bits();
        assert!(scaled.validate().is_err());
    }
    assert_eq!(
        serde_json::from_slice::<Descriptor>(&serde_json::to_vec(&descriptor).unwrap()).unwrap(),
        descriptor
    );
    for mutation in 0..9 {
        let mut invalid = descriptor.clone();
        let visual = invalid.profiles[0].visual.as_mut().unwrap();
        match mutation {
            0 => visual.marker_bones[0].bone = 0,
            1 => visual.marker_bones[0].bone = 1,
            2 => visual.marker_bones[0].bone = 6,
            3 => visual.marker_bones[0].bone = 56,
            4 => visual.marker_bones[0].role = 0,
            5 => visual.marker_bones[0].role = 3,
            6 => visual.marker_bones[0].index = 1,
            7 => visual.marker_bones[0].bind_bits[0] = f32::NAN.to_bits(),
            _ => visual.marker_bones.push(visual.marker_bones[0].clone()),
        }
        assert!(invalid.validate().is_err(), "mutation {mutation}");
    }
    let mut legacy = test_descriptor();
    legacy.profiles[0].visual.as_mut().unwrap().marker_bones = descriptor.profiles[0]
        .visual
        .as_ref()
        .unwrap()
        .marker_bones
        .clone();
    assert!(legacy.validate().is_err());
}

#[test]
fn akimbo_additional_marker_groups_preserve_live_count_separately_from_capacity() {
    let mut visual = test_descriptor().profiles.remove(0).visual.unwrap();
    let mut extra = visual.fx.selectors[0].clone();
    extra.offset += 24;
    extra.role = 3;
    extra.markers = 6;
    visual.fx.selectors.push(extra);
    visual.additional_markers = vec![vec![visual.muzzle_bits, visual.eject_bits]];
    visual.validate().unwrap();
    for mutation in 0..5 {
        let mut invalid = visual.clone();
        match mutation {
            0 => invalid.additional_markers.clear(),
            1 => invalid.additional_markers[0].clear(),
            2 => invalid.additional_markers[0] = vec![invalid.muzzle_bits; 7],
            3 => invalid.additional_markers[0][0][0] = f32::NAN.to_bits(),
            _ => invalid.fx.selectors.last_mut().unwrap().markers = 1,
        }
        assert!(invalid.validate().is_err());
    }
    let mut legacy = test_descriptor();
    legacy.profiles[0].visual = Some(visual);
    assert!(legacy.validate().is_err());
}

#[test]
fn akimbo_generalized_pose_requires_its_own_adapter_and_checked_source_dimensions() {
    let mut descriptor = test_descriptor();
    let profile = &mut descriptor.profiles[0];
    profile.adapter = 10;
    profile.rig = Rig {
        animation_tag: 0x81D40010,
        skeleton_tag: 0x81D40011,
        rig_tag: 0x81D40012,
    };
    let pose = GunPose {
        weapon_bone_count: 6,
        marker_bone: 1,
        marker_bind_bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
        control_count: 76,
        animation_bytes: 0x3500,
    };
    assert!(profile.validate().is_err());
    profile.pose = Some(pose.clone());
    descriptor.validate().unwrap();
    let encoded = serde_json::to_vec(&descriptor).unwrap();
    assert_eq!(
        serde_json::from_slice::<Descriptor>(&encoded).unwrap(),
        descriptor
    );
    for flags in [0, 0x10, 0x14, 0x30, u32::MAX] {
        let mut valid = descriptor.clone();
        valid.profiles[0].barrel.flags = flags;
        valid.validate().unwrap();
    }
    for (bones, marker) in [(5, 2), (6, 3), (8, 5), (26, 12), (56, 54)] {
        let mut valid = descriptor.clone();
        let pose = valid.profiles[0].pose.as_mut().unwrap();
        pose.weapon_bone_count = bones;
        pose.marker_bone = marker;
        pose.control_count = 70 + bones;
        valid.validate().unwrap();
    }
    let mut old = descriptor.clone();
    old.schema_version = 4;
    assert!(old.validate().is_err());
    let mut wrong_adapter = test_descriptor();
    wrong_adapter.profiles[0].pose = Some(pose.clone());
    assert!(wrong_adapter.validate().is_err());
    for (bones, marker, controls, bytes) in [
        (2, 1, 72, 0x3500),
        (57, 1, 127, 0x3500),
        (6, 0, 76, 0x3500),
        (6, 6, 76, 0x3500),
        (6, 5, 76, 0x3500),
        (6, 1, 69, 0x3500),
        (6, 1, 79, 0x3500),
        (6, 1, 76, 0xF8),
        (6, 1, 76, 0x1000008),
        (6, 1, 76, 0x3501),
    ] {
        let invalid = GunPose {
            weapon_bone_count: bones,
            marker_bone: marker,
            control_count: controls,
            animation_bytes: bytes,
            ..pose.clone()
        };
        assert!(invalid.validate().is_err());
    }
    for (field, bits) in [
        (0, f32::NAN.to_bits()),
        (3, 0),
        (4, 3.0f32.to_bits()),
        (7, 0),
    ] {
        let mut invalid = pose.clone();
        invalid.marker_bind_bits[field] = bits;
        assert!(invalid.validate().is_err());
    }
    let mut unknown = serde_json::to_value(&descriptor).unwrap();
    unknown["profiles"][0]["pose"]["unchecked"] = true.into();
    assert!(serde_json::from_value::<Descriptor>(unknown).is_err());
    let text = String::from_utf8(encoded).unwrap();
    let duplicate = text.replace(
        "\"control_count\":76",
        "\"control_count\":76,\"control_count\":76",
    );
    assert_ne!(duplicate, text);
    assert!(serde_json::from_str::<Descriptor>(&duplicate).is_err());
}

#[test]
fn akimbo_descriptor_matches_native_parser_fixture() {
    let encoded = include_str!("native-bindings-v5.json");
    let descriptor: Descriptor = serde_json::from_str(encoded).unwrap();
    descriptor.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&descriptor).unwrap(),
        serde_json::from_str::<serde_json::Value>(encoded).unwrap()
    );
    let mut changed = descriptor.clone();
    changed.profiles[0]
        .visual
        .as_mut()
        .unwrap()
        .fx
        .selectors
        .swap(0, 1);
    assert!(changed.validate().is_err());
    changed = descriptor;
    changed.profiles[0].visual.as_mut().unwrap().fx.tag = 0x80ef262f;
    assert!(changed.validate().is_err());
}

#[test]
fn akimbo_descriptor_is_bounded_strict_and_matches_native_ranges() {
    let descriptor = test_descriptor();
    descriptor.validate().unwrap();
    let mut value = serde_json::to_value(&descriptor).unwrap();
    value["runtime_address"] = 0x12345678u64.into();
    assert!(serde_json::from_value::<Descriptor>(value).is_err());
    for field in ["schema_version", "source_artifacts", "profiles"] {
        let mut value = serde_json::to_value(&descriptor).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Descriptor>(value).is_err());
    }
    for offset in [0, 1, u32::MAX] {
        let mut invalid = descriptor.clone();
        invalid.profiles[0].barrel.root_definition = offset;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = descriptor.clone();
    invalid.profiles[0].barrel.variant = u32::MAX;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles[0].rig.skeleton_tag += 1;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles[0].model_tag = 0x81a2990a;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles = vec![descriptor.profiles[0].clone(); MAX_PROFILES + 1];
    assert!(invalid.validate().is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(FILE_NAME);
    fs::write(&path, vec![b' '; BYTE_LIMIT as usize + 1]).unwrap();
    assert!(Descriptor::read(&path).is_err());
}

#[test]
fn akimbo_gap_contract_requires_matching_schema_and_bounded_explicit_gap() {
    let descriptor = test_descriptor();
    for gap in [50, 360, 800] {
        let mut changed = descriptor.clone();
        changed.profiles[0].gap_mm = gap;
        changed.validate().unwrap();
        let encoded = serde_json::to_vec(&changed).unwrap();
        assert_eq!(
            serde_json::from_slice::<Descriptor>(&encoded).unwrap(),
            changed
        );
    }
    for gap in [0, 49, 801, u16::MAX] {
        let mut invalid = descriptor.clone();
        invalid.profiles[0].gap_mm = gap;
        assert!(invalid.validate().is_err());
    }
    let mut old = descriptor.clone();
    old.schema_version = 1;
    assert!(old.validate().is_err());
    let mut missing = serde_json::to_value(&descriptor).unwrap();
    missing["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("gap_mm");
    assert!(serde_json::from_value::<Descriptor>(missing).is_err());
    for gap in [
        serde_json::json!(-1),
        serde_json::json!(360.5),
        serde_json::json!(65536),
    ] {
        let mut invalid = serde_json::to_value(&descriptor).unwrap();
        invalid["profiles"][0]["gap_mm"] = gap;
        assert!(serde_json::from_value::<Descriptor>(invalid).is_err());
    }
}

#[test]
fn akimbo_reload_policy_is_mandatory_bounded_and_requires_current_schema() {
    for policy in [1, 2, 3] {
        let mut descriptor = test_descriptor();
        descriptor.profiles[0].reload_policy = policy;
        descriptor.validate().unwrap();
        let encoded = serde_json::to_vec(&descriptor).unwrap();
        assert_eq!(
            serde_json::from_slice::<Descriptor>(&encoded).unwrap(),
            descriptor
        );
        descriptor.schema_version = 3;
        assert!(descriptor.validate().is_err());
    }
    for policy in [0, 4, u32::MAX] {
        let mut descriptor = test_descriptor();
        descriptor.profiles[0].reload_policy = policy;
        assert!(descriptor.validate().is_err());
    }
    let mut value = serde_json::to_value(test_descriptor()).unwrap();
    value["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("reload_policy");
    assert!(serde_json::from_value::<Descriptor>(value).is_err());
    for policy in [
        serde_json::json!(null),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("2"),
    ] {
        let mut value = serde_json::to_value(test_descriptor()).unwrap();
        value["profiles"][0]["reload_policy"] = policy;
        assert!(serde_json::from_value::<Descriptor>(value).is_err());
    }
    let encoded = serde_json::to_string(&test_descriptor()).unwrap();
    let duplicate = encoded.replace(
        "\"reload_policy\":3",
        "\"reload_policy\":3,\"reload_policy\":3",
    );
    assert_ne!(encoded, duplicate);
    assert!(serde_json::from_str::<Descriptor>(&duplicate).is_err());
}

#[test]
fn akimbo_package_pins_reject_traversal_duplicates_and_mismatched_generations() {
    let descriptor = test_descriptor();
    let artifact = ArtifactMetadata {
        file_name: crate::package_profile::PARHELION_ASSET_FILE_NAME.into(),
        byte_length: 1,
        sha256: "A".repeat(64),
    };
    descriptor
        .verify_package_metadata(&[], std::slice::from_ref(&artifact))
        .unwrap();
    let mut stale = artifact.clone();
    stale.sha256 = "B".repeat(64);
    assert!(descriptor.verify_package_metadata(&[], &[stale]).is_err());
    for path in [
        "packages/../outside.pkg",
        "packages/sub/file.pkg",
        "packages/C:outside.pkg",
        "outside.pkg",
    ] {
        let mut invalid = descriptor.clone();
        invalid.packages[0].path = path.into();
        assert!(invalid.validate().is_err(), "{path}");
    }
    let mut duplicate = descriptor.clone();
    let mut source = descriptor.packages[0].clone();
    source.path = source
        .path
        .to_uppercase()
        .replacen("PACKAGES/", "packages/", 1);
    duplicate.source_artifacts.push(source);
    assert!(duplicate.validate().is_err());
    let mut outside = descriptor;
    outside.profiles[0].model_tag = tiger_pkg::TagHash::new(0x0AA1, 1).0;
    assert!(outside.verify_package_metadata(&[], &[artifact]).is_err());
}

#[test]
fn akimbo_visual_frames_selectors_and_shared_fx_are_strictly_bounded() {
    let descriptor = test_descriptor();
    for (index, value) in [
        (0, f32::NAN.to_bits()),
        (3, 0),
        (4, 3.0f32.to_bits()),
        (7, 1.05f32.to_bits()),
    ] {
        let mut invalid = descriptor.clone();
        invalid.profiles[0].visual.as_mut().unwrap().muzzle_bits[index] = value;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = descriptor.clone();
    invalid.profiles[0].visual = None;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles[0].visual.as_mut().unwrap().fx.selectors[0].offset = 240;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles[0].visual.as_mut().unwrap().fx.selectors[0].binding = 2;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    invalid.profiles[0].visual.as_mut().unwrap().fx.selectors[0].role = 2;
    assert!(invalid.validate().is_err());
    let mut invalid = descriptor.clone();
    let fx = &mut invalid.profiles[0].visual.as_mut().unwrap().fx;
    fx.selectors.push(fx.selectors[0].clone());
    assert!(invalid.validate().is_err());
    let mut shared = descriptor.clone();
    let mut second = shared.profiles[0].clone();
    second.model_tag += 1;
    second.visual.as_mut().unwrap().muzzle_bits[4] = 0.1f32.to_bits();
    shared.profiles.push(second);
    shared.validate().unwrap();
    shared.profiles[1].visual.as_mut().unwrap().fx.bytes += 8;
    assert!(shared.validate().is_err());
    let mut value = serde_json::to_value(descriptor).unwrap();
    value["profiles"][0]["visual"]["muzzle_bits"]
        .as_array_mut()
        .unwrap()
        .push(0.into());
    assert!(serde_json::from_value::<Descriptor>(value).is_err());
}

#[test]
fn akimbo_capability_is_bound_to_real_runtime_and_executable_bytes_offline() {
    use sundial::package_authoring::{RuntimeBrand, RuntimeSnapshot};
    let directory = tempfile::tempdir().unwrap();
    let packages = directory.path().join("packages");
    fs::create_dir(&packages).unwrap();
    let module = directory.path().join("steam_api64.dll");
    fs::write(&module, b"offline test native module").unwrap();
    fs::write(
        directory.path().join("destiny2.exe"),
        b"offline test executable",
    )
    .unwrap();
    let runtime = RuntimeSnapshot::from_verified_module(RuntimeBrand::Dawn, &module).unwrap();
    assert!(InstalledCapability::read_for_runtime(&packages, &runtime).is_err());
    let path = directory.path().join(CAPABILITY_PATH);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut capability = Capability {
        schema_version: SCHEMA,
        game_build: GAME_BUILD,
        runtime_sha256: runtime.dll_sha256().into(),
        adapter_ids: vec![7],
        max_profiles: MAX_PROFILES as u32,
        descriptor_schema: SCHEMA,
    };
    fs::write(&path, serde_json::to_vec(&capability).unwrap()).unwrap();
    let installed = InstalledCapability::read_for_runtime(&packages, &runtime).unwrap();
    let mut descriptor = test_descriptor();
    descriptor.runtime_sha256 = installed.runtime_sha256.clone();
    descriptor.exe_sha256 = installed.exe_sha256.clone();
    descriptor.verify_identity(&installed).unwrap();
    for old_schema in [1, 2, 3] {
        let mut old = capability.clone();
        old.schema_version = old_schema;
        old.descriptor_schema = old_schema;
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(InstalledCapability::read_for_runtime(&packages, &runtime).is_err());
    }
    fs::write(&path, serde_json::to_vec(&capability).unwrap()).unwrap();
    fs::write(directory.path().join("destiny2.exe"), b"changed executable").unwrap();
    let changed_exe = InstalledCapability::read_for_runtime(&packages, &runtime).unwrap();
    assert!(descriptor.verify_identity(&changed_exe).is_err());
    capability.runtime_sha256 = "0".repeat(64);
    fs::write(&path, serde_json::to_vec(&capability).unwrap()).unwrap();
    assert!(InstalledCapability::read_for_runtime(&packages, &runtime).is_err());
    capability.runtime_sha256 = runtime.dll_sha256().into();
    capability.adapter_ids = vec![4];
    assert!(capability.validate().is_err());
}
