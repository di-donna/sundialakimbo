//! Opt-in native layout evidence. No installed file is modified.
use super::*;
use sundial::package_authoring::entity::{weapon_component_bindings, weapon_entity_assignment};

#[test]
#[ignore = "requires clean SUNDIAL_STOCK_PACKAGES and a fresh SUNDIAL_TEST_ARTIFACTS output"]
fn build_stock_akimbo_candidate() {
    let packages = crate::test_support::stock_packages();
    let catalog = crate::test_support::catalog(packages.parent().unwrap()).unwrap();
    let donors = catalog.weapon_donors();
    let requested =
        std::env::var("SUNDIAL_AKIMBO_DONOR").unwrap_or_else(|_| "IKELOS_SMG_v1.0.1".into());
    let requested_hash = requested
        .strip_prefix("0x")
        .and_then(|hash| u32::from_str_radix(hash, 16).ok());
    let matches = donors
        .iter()
        .filter(|donor| requested_hash.map_or(donor.name == requested, |hash| donor.hash == hash))
        .collect::<Vec<_>>();
    let donor = matches
        .first()
        .unwrap_or_else(|| panic!("No weapon donor {requested:?}"));
    if matches.len() > 1 {
        println!(
            "Multiple source items named {requested:?}; selecting {:08X}. Use 0xHASH to probe an exact item.",
            donor.hash
        );
    }
    let mut recipe = crate::WeaponRecipe::new_weapon("parhelion.akimbo-validation").unwrap();
    recipe.set_donor(donor.hash, donor.name.clone());
    recipe
        .rename_authored_item(format!("{} Akimbo", donor.name))
        .unwrap();
    recipe.overrides.akimbo = Some(Default::default());
    let bundle = build_weapon_project(
        &packages,
        &WeaponProjectSpec {
            weapons: vec![recipe.to_spec().unwrap()],
        },
    )
    .unwrap_or_else(|error| panic!("{} ({:08X}): {error}", donor.name, donor.hash));
    assert_eq!(bundle.plan.akimbo_profiles.len(), 1);
    if requested == "IKELOS_SMG_v1.0.1" {
        let profile = &bundle.plan.akimbo_profiles[0];
        assert_eq!(
            profile.barrel.variant, 13,
            "The native variant excludes the embedded default property"
        );
        assert_eq!(profile.barrel.property_count, 22);
    }
    let output = crate::test_support::artifact_dir("akimbo-candidate");
    assert!(
        !output.exists(),
        "Keep each emitted candidate in a fresh output directory"
    );
    fs::create_dir_all(&output).unwrap();
    bundle.write_new(&output).unwrap();
    fs::write(output.join("recipe.json"), recipe.to_json_pretty().unwrap()).unwrap();
    fs::write(
        output.join("profiles.json"),
        serde_json::to_vec_pretty(&bundle.plan.akimbo_profiles).unwrap(),
    )
    .unwrap();
    println!("Built {} into {}", donor.name, output.display());
}

#[test]
#[ignore = "requires clean SUNDIAL_STOCK_PACKAGES and a fresh SUNDIAL_TEST_ARTIFACTS output"]
fn build_stock_multiweapon_akimbo_candidate() {
    let packages = crate::test_support::stock_packages();
    let catalog = crate::test_support::catalog(packages.parent().unwrap()).unwrap();
    let donors = catalog.weapon_donors();
    let requested =
        std::env::var("SUNDIAL_AKIMBO_DONORS").unwrap_or_else(|_| "0x66BA1A67,0x21CB608D".into());
    let recipes = requested
        .split(',')
        .enumerate()
        .map(|(index, selection)| {
            let selection = selection.trim();
            let hash = selection
                .strip_prefix("0x")
                .map(|value| u32::from_str_radix(value, 16).unwrap());
            let matches = donors
                .iter()
                .filter(|donor| hash.map_or(donor.name == selection, |hash| donor.hash == hash))
                .collect::<Vec<_>>();
            let [donor] = matches.as_slice() else {
                panic!("Select one exact donor for {selection:?}")
            };
            let mut recipe = crate::WeaponRecipe::new_weapon(format!(
                "parhelion.akimbo-multi-validation.{index}"
            ))
            .unwrap();
            recipe.set_donor(donor.hash, donor.name.clone());
            recipe
                .rename_authored_item(format!("{} Akimbo", donor.name))
                .unwrap();
            recipe.overrides.akimbo = Some(Default::default());
            recipe
        })
        .collect::<Vec<_>>();
    assert!((2..=crate::weapon::akimbo::deployment::MAX_PROFILES).contains(&recipes.len()));
    let bundle = build_weapon_project(
        &packages,
        &WeaponProjectSpec {
            weapons: recipes
                .iter()
                .map(|recipe| recipe.to_spec().unwrap())
                .collect(),
        },
    )
    .unwrap();
    let profiles = &bundle.plan.akimbo_profiles;
    assert_eq!(profiles.len(), recipes.len());
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.model_tag)
            .collect::<BTreeSet<_>>()
            .len(),
        recipes.len(),
        "Native model identities must remain independent in one generation"
    );
    let output = crate::test_support::artifact_dir("akimbo-multi-candidate");
    assert!(!output.exists(), "Choose a fresh artifact directory");
    fs::create_dir_all(&output).unwrap();
    let paths = bundle.write_new(&output).unwrap();
    let view = crate::workflow::FilteredPackageView::create(&packages, &[]).unwrap();
    for path in paths {
        view.add_overlay(&path).unwrap();
    }
    let manager = open_manager(view.path()).unwrap();
    let pins = profiles
        .iter()
        .map(|profile| {
            let owner = manager.read_tag(TagHash(profile.model_tag)).unwrap();
            let definition = crate::tag_payload::relative_target(&owner, 0x18).unwrap();
            assert_eq!(read_u32(&owner, definition - 4).unwrap(), 0x808072BD);
            let model = manager
                .read_tag(TagHash(read_u32(&owner, definition + 0x1DC).unwrap()))
                .unwrap();
            std::array::from_fn::<_, 4, _>(|axis| read_u32(&model, 0x60 + axis * 4).unwrap())
        })
        .collect::<BTreeSet<_>>();
    assert!(
        pins.len() >= 2,
        "The selected donors must exercise distinct effect selectors"
    );
    for header in [
        0x80FEBA62, 0x80FEBA67, 0x80FEBA6D, 0x80FEBA73, 0x80FEBA81, 0x80FEBA87, 0x80FEBA60,
        0x80FEBA66, 0x80FEBA6B, 0x80FEBA71, 0x80FEBA7F, 0x80FEBA86,
    ] {
        let code = manager.get_entry(TagHash(header)).unwrap().reference;
        let program =
            crate::dxbc::Program::read_vertex(&manager.read_tag(TagHash(code)).unwrap()).unwrap();
        let selectors = program
            .instructions
            .iter()
            .filter(|row| row.opcode() == 32)
            .map(|row| row.parsed().unwrap())
            .filter_map(|row| match row.args.as_slice() {
                [_, buffer, literal] if buffer.cb(11, 5) && literal.kind() == 4 => {
                    <[u32; 4]>::try_from(&literal.0[1..]).ok()
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert!(
            pins.is_subset(&selectors),
            "Shared effect {header:08X} must dispatch both weapons"
        );
    }
    fs::write(
        output.join("profiles.json"),
        serde_json::to_vec_pretty(profiles).unwrap(),
    )
    .unwrap();
    for (index, recipe) in recipes.iter().enumerate() {
        fs::write(
            output.join(format!("recipe-{index}.json")),
            recipe.to_json_pretty().unwrap(),
        )
        .unwrap();
    }
    drop(manager);
    view.close().unwrap();
    println!(
        "Built {} independently registered weapons into {}",
        profiles.len(),
        output.display()
    );
}

#[test]
#[ignore = "requires SUNDIAL_STOCK_PACKAGES and SUNDIAL_TEST_ARTIFACTS"]
fn survey_stock_akimbo_layouts() {
    let packages = crate::test_support::stock_packages();
    let sources = sources::load_project_sources(&packages).unwrap();
    let mut report = Vec::new();
    let mut index = 0;
    while let Some(pattern) =
        sandbox_pattern_identity_at(&sources.stock_sandbox_patterns, index).unwrap()
    {
        index += 1;
        let Some(tag) = weapon_entity_assignment(
            &sources.stock_entity_assignments,
            pattern.pattern_global_id_hash,
        )
        .unwrap() else {
            continue;
        };
        let entity = sources.manager.read_tag(TagHash(tag)).unwrap();
        let mut bindings = Vec::new();
        for key in [
            0xC18BD28D, 0xD5A123FF, 0xEC711FA3, 0x39AFD7D3, 0xB1AAA2CB, 0xB9CAA3BC, 0x7E4A5223,
            0x2F981564, 0xD3A5500E, 0x1C80DD4A,
        ] {
            for binding in weapon_component_bindings(&entity, key).unwrap_or_default() {
                let owner = sources
                    .manager
                    .read_tag(TagHash(binding.owner_tag))
                    .unwrap();
                let data = crate::tag_payload::relative_target(&owner, 0x18).unwrap();
                bindings.push(serde_json::json!({"key":format!("{key:08X}"),"owner":format!("{:08X}",binding.owner_tag),"class":format!("{:08X}",binding.concrete_class),"offset":binding.resource_offset,"count":binding.resource_count,"data":data,"data_class":format!("{:08X}",read_u32(&owner,data-4).unwrap()),"size":owner.len()}));
            }
        }
        let rig = crate::weapon::animations::arms_rig(
            &sources.manager,
            &entity,
            Some(pattern.weapon_content_group_hash),
        );
        let rig = match rig {
            Ok(rig) => {
                let bytes = sources.manager.read_tag(TagHash(rig.entity_tag)).unwrap();
                let (count, _, rows, _) = crate::tag_payload::array_at(&bytes, 0x10).unwrap();
                let mut owners = Vec::new();
                for n in 0..count {
                    let tag = read_u32(&bytes, rows + n * 12).unwrap();
                    let bytes = sources.manager.read_tag(TagHash(tag)).unwrap();
                    let data = crate::tag_payload::relative_target(&bytes, 0x18).unwrap();
                    owners.push(serde_json::json!({"tag":format!("{tag:08X}"),"data":data,"class":format!("{:08X}",read_u32(&bytes,data-4).unwrap()),"size":bytes.len()}));
                }
                serde_json::json!({"entity":format!("{:08X}",rig.entity_tag),"owners":owners})
            }
            Err(error) => serde_json::json!({"error":error.to_string()}),
        };
        report.push(serde_json::json!({"item":format!("{:08X}",pattern.item_hash),"entity":format!("{tag:08X}"),"group":format!("{:08X}",pattern.weapon_content_group_hash),"bindings":bindings,"arms":rig}));
    }
    assert!(!report.is_empty());
    let output = crate::test_support::artifact_dir("akimbo-layout-survey");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("layouts.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "Surveyed {} stock runtime layouts into {}",
        report.len(),
        output.display()
    );
}
