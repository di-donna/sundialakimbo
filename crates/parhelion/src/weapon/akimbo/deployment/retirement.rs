//! Recognition of installed historical bytes for backup/removal, never staging or activation.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyVisual {
    muzzle_bits: [u32; 8],
    eject_bits: [u32; 8],
    tracer_tag: u32,
    tracer_definition: u32,
    fx: Fx,
}
impl From<LegacyVisual> for Visual {
    fn from(old: LegacyVisual) -> Self {
        Self {
            muzzle_bits: old.muzzle_bits,
            eject_bits: old.eject_bits,
            additional_markers: Vec::new(),
            marker_bones: Vec::new(),
            tracer_tag: old.tracer_tag,
            tracer_definition: old.tracer_definition,
            fx: old.fx,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProfileV1 {
    model_tag: u32,
    adapter: u32,
    rig: Rig,
    barrel: Barrel,
    #[serde(default)]
    visual: Option<LegacyVisual>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProfileV2 {
    model_tag: u32,
    adapter: u32,
    gap_mm: u16,
    rig: Rig,
    barrel: Barrel,
    #[serde(default)]
    visual: Option<LegacyVisual>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProfileV3 {
    model_tag: u32,
    adapter: u32,
    gap_mm: u16,
    reload_policy: u32,
    rig: Rig,
    barrel: Barrel,
    #[serde(default)]
    visual: Option<LegacyVisual>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDescriptor<P> {
    schema_version: u32,
    game_build: u32,
    generation: String,
    runtime_sha256: String,
    exe_sha256: String,
    packages: Vec<Package>,
    source_artifacts: Vec<Package>,
    profiles: Vec<P>,
}

impl<P> LegacyDescriptor<P> {
    fn validate(self, schema: u32, upgrade: impl FnMut(P) -> Profile) -> Result<(), String> {
        if self.schema_version != schema {
            return Err("Unsupported installed akimbo descriptor schema".into());
        }
        // Reuse current structural checks, but do not expose this temporary descriptor
        // for activation or write it to disk. Preserve each historical policy.
        let profiles = self.profiles.into_iter().map(upgrade).collect::<Vec<_>>();
        if profiles
            .iter()
            .any(|profile| profile.barrel.variant >= profile.barrel.property_count)
        {
            return Err("Historical akimbo descriptors require an indexed content variant".into());
        }
        Descriptor {
            schema_version: SCHEMA,
            game_build: self.game_build,
            generation: self.generation,
            runtime_sha256: self.runtime_sha256,
            exe_sha256: self.exe_sha256,
            packages: self.packages,
            source_artifacts: self.source_artifacts,
            profiles,
        }
        .validate()
    }
}

pub(super) fn validate(bytes: &[u8]) -> Result<(), String> {
    // Every reader deserializes structs directly. A Value-based migration would silently
    // collapse duplicate keys before the strict reader could reject them.
    if let Ok(current) = serde_json::from_slice::<Descriptor>(bytes)
        && current.schema_version == SCHEMA
    {
        return current.validate();
    }
    if let Ok(legacy) = serde_json::from_slice::<LegacyDescriptor<LegacyProfileV3>>(bytes) {
        let schema = legacy.schema_version;
        if !matches!(schema, 3 | 4) {
            return Err("Unsupported installed akimbo descriptor schema".into());
        }
        let maximum_policy = if schema == 4 { 3 } else { 2 };
        if legacy
            .profiles
            .iter()
            .any(|profile| !(1..=maximum_policy).contains(&profile.reload_policy))
        {
            return Err("Unsupported reload policy in historical akimbo descriptor".into());
        }
        return legacy.validate(schema, |profile| Profile {
            model_tag: profile.model_tag,
            adapter: profile.adapter,
            pose: None,
            gap_mm: profile.gap_mm,
            reload_policy: profile.reload_policy,
            rig: profile.rig,
            barrel: profile.barrel,
            visual: profile.visual.map(Into::into),
        });
    }
    if let Ok(legacy) = serde_json::from_slice::<LegacyDescriptor<LegacyProfileV2>>(bytes) {
        return legacy.validate(2, |profile| Profile {
            model_tag: profile.model_tag,
            adapter: profile.adapter,
            pose: None,
            gap_mm: profile.gap_mm,
            reload_policy: 1,
            rig: profile.rig,
            barrel: profile.barrel,
            visual: profile.visual.map(Into::into),
        });
    }
    serde_json::from_slice::<LegacyDescriptor<LegacyProfileV1>>(bytes)
        .map_err(|error| format!("Invalid installed akimbo descriptor: {error}"))?
        .validate(1, |profile| Profile {
            model_tag: profile.model_tag,
            adapter: profile.adapter,
            pose: None,
            gap_mm: 360,
            reload_policy: 1,
            rig: profile.rig,
            barrel: profile.barrel,
            visual: profile.visual.map(Into::into),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn akimbo_historical_descriptors_do_not_inherit_default_content_semantics() {
        for schema in [1, 2, 3, 4] {
            let mut value = legacy(schema);
            value["profiles"][0]["barrel"]["variant"] = u32::MAX.into();
            assert!(validate(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    fn legacy(schema: u32) -> serde_json::Value {
        let mut value = serde_json::to_value(test_descriptor()).unwrap();
        value["schema_version"] = schema.into();
        let profile = value["profiles"][0].as_object_mut().unwrap();
        if schema < 3 {
            profile.remove("reload_policy");
        } else {
            profile.insert("reload_policy".into(), 2.into());
        }
        if schema == 1 {
            profile.remove("gap_mm");
        }
        value
    }

    #[test]
    fn akimbo_historical_visuals_reject_new_marker_fields_even_when_empty() {
        for schema in [1, 2, 3, 4] {
            for field in ["additional_markers", "marker_bones"] {
                let mut value = legacy(schema);
                value["profiles"][0]["visual"][field] = serde_json::json!([]);
                assert!(validate(&serde_json::to_vec(&value).unwrap()).is_err());
            }
        }
    }

    #[test]
    fn akimbo_legacy_retirement_hashes_original_bytes_without_enabling_staging() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE_NAME);
        for schema in [1, 2, 3, 4] {
            let mut bytes = serde_json::to_vec_pretty(&legacy(schema)).unwrap();
            bytes.extend_from_slice(b"\r\n  ");
            fs::write(&path, &bytes).unwrap();
            let metadata = Descriptor::validate_installed_file(&path).unwrap();
            let original = digest_bytes(&bytes);
            assert_eq!(metadata.byte_length, original.byte_length);
            assert_eq!(metadata.sha256, original.sha256);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(Descriptor::read(&path).is_err());
        }

        let current = serde_json::to_vec(&test_descriptor()).unwrap();
        fs::write(&path, &current).unwrap();
        assert_eq!(
            Descriptor::validate_installed_file(&path).unwrap().sha256,
            digest_bytes(&current).sha256
        );
        Descriptor::read(&path).unwrap();
    }

    #[test]
    fn akimbo_legacy_retirement_rejects_unknown_duplicate_and_malformed_data() {
        let original = legacy(1);
        let mut invalid = Vec::new();
        for field in ["gap_mm", "reload_policy", "unexpected"] {
            let mut changed = original.clone();
            changed["profiles"][0][field] = 360.into();
            invalid.push(changed);
        }
        let mut changed = original.clone();
        changed["unexpected"] = true.into();
        invalid.push(changed);
        for version in [0, 2, 3] {
            let mut changed = original.clone();
            changed["schema_version"] = version.into();
            invalid.push(changed);
        }
        let mut changed = original.clone();
        changed["profiles"][0]["barrel"]["core_size"] = 1.into();
        invalid.push(changed);
        let mut changed = original.clone();
        changed["profiles"][0]["visual"] = serde_json::Value::Null;
        invalid.push(changed);
        let mut changed = original.clone();
        changed["packages"][0]["path"] = "packages/../foreign.pkg".into();
        invalid.push(changed);
        for changed in invalid {
            assert!(validate(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        let encoded = serde_json::to_string(&original).unwrap();
        for duplicate in [
            encoded.replace(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
            ),
            encoded.replace("\"adapter\":7", "\"adapter\":7,\"adapter\":7"),
            encoded.replace("\"flags\":16", "\"flags\":16,\"flags\":16"),
        ] {
            assert_ne!(duplicate, encoded);
            assert!(validate(duplicate.as_bytes()).is_err());
        }
    }

    #[test]
    fn akimbo_schema_two_retirement_requires_gap_and_rejects_policy_or_duplicate_keys() {
        validate(include_bytes!("native-bindings-v2.json")).unwrap();
        for gap in [50, 360, 800] {
            let mut valid = legacy(2);
            valid["profiles"][0]["gap_mm"] = gap.into();
            validate(&serde_json::to_vec(&valid).unwrap()).unwrap();
        }
        let original = legacy(2);
        for gap in [
            serde_json::Value::Null,
            serde_json::json!(49),
            serde_json::json!(801),
        ] {
            let mut changed = original.clone();
            changed["profiles"][0]["gap_mm"] = gap;
            assert!(validate(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        let mut changed = original.clone();
        changed["profiles"][0]
            .as_object_mut()
            .unwrap()
            .remove("gap_mm");
        assert!(validate(&serde_json::to_vec(&changed).unwrap()).is_err());
        for field in ["reload_policy", "unexpected"] {
            let mut changed = original.clone();
            changed["profiles"][0][field] = 1.into();
            assert!(validate(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        let encoded = serde_json::to_string(&original).unwrap();
        let duplicate = encoded.replace("\"gap_mm\":360", "\"gap_mm\":360,\"gap_mm\":360");
        assert_ne!(encoded, duplicate);
        assert!(validate(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn akimbo_schema_three_retirement_keeps_only_its_original_policies() {
        validate(include_bytes!("native-bindings-v3.json")).unwrap();
        let original = legacy(3);
        for policy in [1, 2] {
            let mut valid = original.clone();
            valid["profiles"][0]["reload_policy"] = policy.into();
            validate(&serde_json::to_vec(&valid).unwrap()).unwrap();
        }
        for policy in [0, 3, u32::MAX] {
            let mut invalid = original.clone();
            invalid["profiles"][0]["reload_policy"] = policy.into();
            assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        for field in ["gap_mm", "reload_policy"] {
            let mut invalid = original.clone();
            invalid["profiles"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = original.clone();
        invalid["profiles"][0]["unexpected"] = true.into();
        assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let encoded = serde_json::to_string(&original).unwrap();
        let duplicate = encoded.replace(
            "\"reload_policy\":2",
            "\"reload_policy\":2,\"reload_policy\":2",
        );
        assert_ne!(encoded, duplicate);
        assert!(validate(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn akimbo_schema_four_retirement_preserves_lower_magazine_but_rejects_new_pose_data() {
        validate(include_bytes!("native-bindings-v4.json")).unwrap();
        let mut original = legacy(4);
        original["profiles"][0]["reload_policy"] = 3.into();
        validate(&serde_json::to_vec(&original).unwrap()).unwrap();
        for policy in [0, 4, u32::MAX] {
            let mut invalid = original.clone();
            invalid["profiles"][0]["reload_policy"] = policy.into();
            assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        for pose in [serde_json::Value::Null, serde_json::json!({})] {
            let mut invalid = original.clone();
            invalid["profiles"][0]["pose"] = pose;
            assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = original.clone();
        invalid["profiles"][0]["adapter"] = 10.into();
        assert!(validate(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let encoded = serde_json::to_string(&original).unwrap();
        let duplicate = encoded.replace(
            "\"reload_policy\":3",
            "\"reload_policy\":3,\"reload_policy\":3",
        );
        assert_ne!(encoded, duplicate);
        assert!(validate(duplicate.as_bytes()).is_err());
    }
}
