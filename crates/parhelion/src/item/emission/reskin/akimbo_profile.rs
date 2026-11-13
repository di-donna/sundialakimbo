//! Derive the native adapter binding from the final compiled runtime, never the donor's name.
use super::*;
use crate::tag_payload::{read_u64, relative_target};
use crate::weapon::akimbo::deployment::{Barrel, Profile, Rig};
use sundial::package_authoring::entity::{weapon_component_bindings, weapon_entity_assignment};

mod core;
mod stats;

/// Read pending private allocations before stock, so edits cannot pass using stale donor data.
pub(super) fn pending(
    manager: &PackageManager,
    emission: &PackageEmission,
    tag: u32,
) -> AuthoringResult<Vec<u8>> {
    let hash = TagHash(tag);
    for package in &emission.asset_packages.packages {
        if package.id == hash.pkg_id() {
            return package
                .tags
                .get(hash.entry_index() as usize)
                .map(|node| node.payload.clone())
                .ok_or_else(|| invalid("Akimbo runtime references a missing private asset"));
        }
    }
    let allocator =
        AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
    for (index, node) in emission.private_perk_runtime_new_tags.iter().enumerate() {
        if allocator.assigned_tag(index, "Akimbo runtime", "owner")? == hash {
            return Ok(node.payload.clone());
        }
    }
    read(manager, tag)
}

pub(super) fn assess(
    manager: &PackageManager,
    emission: &PackageEmission,
    spec: &WeaponCloneSpec,
) -> AuthoringResult<(Profile, Vec<u32>)> {
    if spec.presentation_donor.is_some()
        || spec.render_gear_donor.is_some()
        || spec.overrides.animation_donor.is_some()
        || !spec.overrides.animation_actions.is_empty()
        || !spec.overrides.additional_behaviors.is_empty()
        || spec.overrides.variable_damage.is_some()
        || spec.overrides.held_offset.is_some()
        || !spec.overrides.marker_offsets.is_empty()
    {
        return Err(invalid(
            "This akimbo adapter requires the weapon's own appearance, rig, markers and behavior; those combinations need a separate compatibility adapter",
        ));
    }
    #[cfg(feature = "d2-model-importer")]
    if spec.overrides.imported_graph.is_some() {
        return Err(invalid(
            "Akimbo currently requires a native weapon appearance",
        ));
    }
    let pattern = sandbox_pattern_identity(&emission.sandbox_patterns, spec.identity.item_hash)
        .map_err(invalid)?
        .ok_or_else(|| invalid("Akimbo weapon has no final runtime pattern"))?;
    let tag =
        weapon_entity_assignment(&emission.entity_assignments, pattern.pattern_global_id_hash)
            .map_err(invalid)?
            .ok_or_else(|| invalid("Akimbo weapon has no final runtime entity"))?;
    let entity = pending(manager, emission, tag)?;
    let source_tags = std::cell::RefCell::new(BTreeSet::from([tag]));
    let read = |tag| {
        source_tags.borrow_mut().insert(tag);
        pending(manager, emission, tag)
    };
    let binding = |key| {
        let bindings = weapon_component_bindings(&entity, key).map_err(invalid)?;
        match bindings.as_slice() {
            [binding] if binding.resource_count == 1 => Ok(*binding),
            _ => Err(invalid(format!(
                "Akimbo requires one resource for component 0x{key:08X}"
            ))),
        }
    };
    let controller = binding(0x39afd7d3)?;
    unique_core_owner(&entity, controller.owner_tag)?;
    let core = read(controller.owner_tag)?;
    let core_layout = core::inspect(&core, controller.owner_tag)?;
    // These are the audited independent magazine adapter's native interfaces, not item IDs.
    for (key, class, offset) in [
        (0xc18bd28d, 0x808043da, 0x6f0),
        (0xd5a123ff, 0x8080388a, 0xcd0),
        (0xec711fa3, 0x80803889, 0xdf0),
        (0x39afd7d3, core_layout.root_kind.1, 0x80),
        (0xb1aaa2cb, 0x80803a0f, 0x2550),
        (0xb9caa3bc, core_layout.reload_kind.1, 0x2790),
    ] {
        let actual = binding(key)?;
        if actual.owner_tag != controller.owner_tag
            || actual.concrete_class != class
            || actual.resource_offset != offset
        {
            return Err(invalid(format!(
                "Akimbo component 0x{key:08X} does not match the independent magazine adapter's ownership and layout"
            )));
        }
    }
    if weapon_component_bindings(&entity, 0x7e4a5223).is_ok_and(|bindings| !bindings.is_empty()) {
        return Err(invalid("Charged triggers need a separate akimbo adapter"));
    }
    // The generic host derives ordinary or tube reload storage from this exact
    // typed core. Its tube scheduler reads and pins the source opening, refill
    // quantum and closing properties before publishing independent magazines.
    // CC9460's 0x04 bit tests the live bullet count; it does not establish a
    // burst by itself. 0x08 retains the native pattern-state path in 181C8B0,
    // and 0x10 is a native firing-mode default. The grouped emitter preserves
    // these arguments. Continuous 0x20 and other unaudited flags stay gated.
    if !matches!(core_layout.flags, 0 | 0x04 | 0x10 | 0x14 | 0x18 | 0x1C)
        || core_layout.delayed_emission != 0
    {
        return Err(invalid(
            "Continuous or delayed firing requires independent native event ownership",
        ));
    }
    let root_definition = core_layout.definitions[0] as u64;
    let barrel_definition = core_layout.definitions[3] as u64;
    checked_pattern(
        sundial::package_authoring::entity::spread::read_pattern(&core, binding(0xec711fa3)?)
            .map_err(invalid)?,
    )?;
    let content = crate::weapon::behavior::content_with(&entity, read)?;
    let content_definition = read_u64(&content.owner, content.resource + 8)?;
    let content_binding = binding(0x5f0dd954)?;
    // The engine connects the root's content handle from the entity binding at load time;
    // the serialized root slot is deliberately empty. Validate the bound resource itself.
    if content_binding.concrete_class != crate::weapon::ammo::CONTENT_INSTANCE
        || content.resource != 0x80
        || read_u32(&content.owner, content.resource)? != content.owner_tag
        || read_u32(&content.owner, content.resource + 4)? != 0x80803ac9
        || read_u64(&content.owner, 0)? != content.owner.len() as u64
    {
        return Err(invalid(format!(
            "Akimbo content ownership differs from the native firing adapter: binding={:08X}, resource={:X}, owner={:08X}, self={:08X}, kind={:08X}, declared={}, bytes={}",
            content_binding.concrete_class,
            content.resource,
            content.owner_tag,
            read_u32(&content.owner, content.resource)?,
            read_u32(&content.owner, content.resource + 4)?,
            read_u64(&content.owner, 0)?,
            content.owner.len()
        )));
    }
    let variant = selected_variant(&content, pattern.weapon_content_group_hash)?;
    let arms = crate::weapon::animations::arms_rig(
        manager,
        &entity,
        Some(pattern.weapon_content_group_hash),
    )?;
    unchanged_arms(&arms.source_tags, read, |tag| self::read(manager, tag))?;
    source_tags
        .borrow_mut()
        .extend(arms.source_tags.iter().copied());
    let (count, _, rows, _) = array(&arms.entity, 0x10)?;
    let mut owners = BTreeMap::new();
    for index in 0..count {
        let tag = read_u32(&arms.entity, rows + index * 12)?;
        let bytes = read(tag)?;
        let data = relative_target(&bytes, 0x18)?;
        if data < 4 {
            return Err(invalid("Akimbo arms resource has no definition"));
        }
        let class = read_u32(&bytes, data - 4)?;
        if [0x808036cf, 0x80808546, 0x80808f8f].contains(&class)
            && owners.insert(class, tag).is_some()
        {
            return Err(invalid("Akimbo arms rig has ambiguous component owners"));
        }
    }
    let tag = |class| {
        owners
            .get(&class)
            .copied()
            .ok_or_else(|| invalid("Akimbo arms rig is incomplete"))
    };
    let rig = Rig {
        animation_tag: tag(0x808036cf)?,
        skeleton_tag: tag(0x80808546)?,
        rig_tag: tag(0x80808f8f)?,
    };
    // Preserve adapter 7's audited data and behavior. Other guns receive a private marker
    // channel whose full skeleton/control layout is validated, rather than borrowing a bone.
    let rounds = stats::selected_count(
        manager,
        emission,
        spec.identity.item_hash,
        &entity,
        pattern.weapon_translation_group_hash,
        read,
    )?;
    let legacy = rounds <= 1
        && rig.animation_tag == 0x8152c7cd
        && rig.skeleton_tag == 0x80bc1000
        && rig.rig_tag == 0x80bc1001
        && core::layout(&core, controller.owner_tag).is_ok();
    if legacy && variant.index == u32::MAX {
        return Err(invalid(
            "The original SMG akimbo adapter requires an indexed content variant",
        ));
    }
    let adapter = firing_adapter(
        legacy,
        rounds,
        variant.burst_override,
        core_layout.reload_kind == (0x80803845, 0x80803844),
        crate::tag_payload::read_u8(&core, core_layout.definitions[2] + 0x5a)? == 0,
    )?;
    let pose = if legacy {
        None
    } else {
        Some(crate::weapon::akimbo::visual::pose::prepare(&entity, &arms.entity, read)?.pose)
    };
    let narrow = |value: u64| {
        u32::try_from(value).map_err(|_| invalid("Akimbo native field exceeds 32 bits"))
    };
    Ok((
        Profile {
            model_tag: 0,
            adapter,
            pose,
            reload_policy: spec.overrides.akimbo.as_ref().map_or(
                crate::weapon::akimbo::Reload::LowerMagazine.native_policy(),
                |intent| intent.reload.native_policy(),
            ),
            gap_mm: spec
                .overrides
                .akimbo
                .as_ref()
                .map_or(crate::weapon::akimbo::DEFAULT_GAP_MM, |intent| {
                    intent.gap_mm
                }),
            rig,
            visual: None,
            barrel: Barrel {
                core_tag: controller.owner_tag,
                content_tag: content.owner_tag,
                root_definition: narrow(root_definition)?,
                barrel_definition: narrow(barrel_definition)?,
                content_definition: narrow(content_definition)?,
                core_size: narrow(core.len() as u64)?,
                content_size: narrow(content.owner.len() as u64)?,
                property_count: narrow(content.blocks.len().saturating_sub(1) as u64)?,
                variant: variant.index,
                projectile: variant.projectile,
                flags: core_layout.flags,
            },
        },
        source_tags.into_inner().into_iter().collect(),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectedVariant {
    index: u32,
    projectile: u32,
    burst_override: u32,
}

fn firing_adapter(
    legacy: bool,
    rounds: u16,
    override_count: u32,
    ordinary: bool,
    digital: bool,
) -> AuthoringResult<u32> {
    if rounds <= 1 && override_count <= 1 {
        return Ok(if legacy { 7 } else { 10 });
    }
    // The first finite adapter proves identical serialized min/max columns;
    // runtime rechecks all four live values and the selected content override.
    // A content override cannot turn an unbounded/one-round maximum into a burst.
    if !legacy
        && (2..=64).contains(&rounds)
        && (override_count == 0 || override_count == u32::from(rounds))
        && ordinary
        && digital
    {
        return Ok(11);
    }
    Err(invalid(
        "Finite akimbo bursts require matching 2–64-round firing columns, digital primary input and ordinary magazine reloads",
    ))
}

fn selected_variant(
    content: &crate::weapon::behavior::Content,
    group: u32,
) -> AuthoringResult<SelectedVariant> {
    let selected = crate::weapon::akimbo::content::select(content, group)?;
    let block = selected.block;
    checked_pattern(
        sundial::package_authoring::entity::spread::read_pattern_at(
            &content.owner,
            content.owner_tag,
            block + 0x110,
        )
        .map_err(invalid)?,
    )?;
    // The native selector at CB9440 replaces input 22 with this row override when
    // nonzero. Checking stat translations alone would miss content-owned bursts.
    let burst_override = read_u32(&content.owner, block + 0x140)?;
    if burst_override > 64 {
        return Err(invalid(
            "Content-defined burst exceeds the bounded native queue",
        ));
    }
    let projectile = read_u32(&content.owner, block + 0xf0)?;
    if projectile != u32::MAX && (projectile < 0x8000_0000 || projectile == 0x811C_9DC5) {
        return Err(invalid(
            "Akimbo content selects an invalid projectile identity",
        ));
    }
    // The native content route may override the barrel's fallback projectile. Keep that exact
    // identity in the binding; visual assessment resolves and validates the selected graph.
    Ok(SelectedVariant {
        index: selected.index,
        projectile,
        burst_override,
    })
}

fn checked_pattern(
    pattern: Option<sundial::package_authoring::entity::spread::Pattern>,
) -> AuthoringResult<()> {
    // The native event retains every original origin and calls the stock
    // grouped emitter once. Its checked scratch storage holds at most 64
    // origins; pellets are never interpreted as burst rounds or ammo cost.
    if let Some(pattern) = pattern
        && pattern.validate().map_err(invalid)? > 64
    {
        return Err(invalid(
            "Akimbo firing supports at most 64 native pellet origins",
        ));
    }
    Ok(())
}

/// The native magazine lookup walks the complete component list, including unbound rows.
fn unique_core_owner(entity: &[u8], core: u32) -> AuthoringResult<()> {
    let (count, _, rows, class) = array(entity, 0x10)?;
    if !(1..=128).contains(&count) || class != 0x80809C04 {
        return Err(invalid(
            "Akimbo runtime component table differs from the native adapter",
        ));
    }
    let mut matches = 0;
    for index in 0..count {
        matches += usize::from(read_u32(entity, rows + index * 12)? == core);
    }
    if matches != 1 {
        return Err(invalid(
            "Akimbo runtime must contain exactly one native core owner",
        ));
    }
    Ok(())
}

/// `arms_rig` is a stock graph reader. Every byte it used must equal the final pending graph;
/// a private attachment that it cannot resolve already fails before reaching this comparison.
fn unchanged_arms(
    tags: &[u32],
    pending: impl Fn(u32) -> AuthoringResult<Vec<u8>>,
    stock: impl Fn(u32) -> AuthoringResult<Vec<u8>>,
) -> AuthoringResult<()> {
    // This shared first-person controller is consumed by the native pose hook as well.
    for tag in tags
        .iter()
        .copied()
        .chain([0x80BDE357])
        .collect::<BTreeSet<_>>()
    {
        if pending(tag)? != stock(tag)? {
            return Err(invalid(format!(
                "Akimbo first-person source 0x{tag:08X} was edited; the native pose adapter requires its audited graph"
            )));
        }
    }
    Ok(())
}

pub(super) fn finish_visual(
    manager: &PackageManager,
    emission: &mut PackageEmission,
    spec: &WeaponCloneSpec,
    gear_entities: &[u32],
    profile: &mut Profile,
) -> AuthoringResult<()> {
    let pattern = sandbox_pattern_identity(&emission.sandbox_patterns, spec.identity.item_hash)
        .map_err(invalid)?
        .ok_or_else(|| invalid("Akimbo final runtime pattern is absent"))?;
    let tag =
        weapon_entity_assignment(&emission.entity_assignments, pattern.pattern_global_id_hash)
            .map_err(invalid)?
            .ok_or_else(|| invalid("Akimbo final runtime entity is absent"))?;
    let source_tags = std::cell::RefCell::new(BTreeSet::new());
    let read = |tag| {
        source_tags.borrow_mut().insert(tag);
        pending(manager, emission, tag)
    };
    let entity = read(tag)?;
    profile.visual = Some(crate::weapon::akimbo::visual::assess_visual(
        &entity,
        pattern.weapon_content_group_hash,
        gear_entities,
        profile.pose.as_ref(),
        read,
    )?);
    emission.akimbo_source_tags.extend(source_tags.into_inner());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn akimbo_finite_bursts_require_matching_native_counts_and_input_contract() {
        for rounds in 0..=65 {
            for selected in 0..=65 {
                let actual = firing_adapter(false, rounds, selected, true, true);
                let expected = if rounds <= 1 && selected <= 1 {
                    Some(10)
                } else if (2..=64).contains(&rounds)
                    && (selected == 0 || selected == u32::from(rounds))
                {
                    Some(11)
                } else {
                    None
                };
                assert_eq!(actual.ok(), expected, "rounds={rounds}, content={selected}");
            }
        }
        for legacy in [false, true] {
            assert_eq!(
                firing_adapter(legacy, 1, 0, false, false).unwrap(),
                if legacy { 7 } else { 10 }
            );
        }
        for rounds in 2..=64 {
            assert!(firing_adapter(true, rounds, 0, true, true).is_err());
            assert!(firing_adapter(false, rounds, 0, false, true).is_err());
            assert!(firing_adapter(false, rounds, 0, true, false).is_err());
        }
    }

    #[test]
    fn akimbo_content_pattern_keeps_pellets_separate_from_ammunition() {
        use crate::tag_payload::{append_native_array, write_u64};
        use sundial::package_authoring::entity::spread::read_pattern_at;
        const TAG: u32 = 0x8152AC56;
        let mut bytes = vec![0; 0x300];
        let pointer = 0x110;
        let pattern = 0x180;
        write_u64(&mut bytes, pointer, (pattern - pointer) as u64).unwrap();
        write_u32(&mut bytes, pattern - 4, 0x8080888D).unwrap();
        for ordinal in 0..3 {
            let at = pattern + ordinal * 24;
            for (offset, value) in [
                (0, 0x8080888D),
                (4, ordinal as u32),
                (8, TAG),
                (12, 0x8080888D),
            ] {
                write_u32(&mut bytes, at + offset, value).unwrap();
            }
            write_u64(&mut bytes, at + 16, pattern as u64).unwrap();
        }
        let mut rings = vec![0; 40];
        for (row, pellets) in [(0, 1), (20, 12)] {
            write_u32(&mut rings, row + 4, 1.0f32.to_bits()).unwrap();
            write_u32(&mut rings, row + 12, pellets).unwrap();
            write_u32(&mut rings, row + 16, 1.0f32.to_bits()).unwrap();
        }
        append_native_array(&mut bytes, pattern + 0x48, 0x8080888F, 2, &rings).unwrap();
        write_u32(&mut bytes, pattern + 0x58, 1.0f32.to_bits()).unwrap();
        write_u32(&mut bytes, pattern + 0x60, 13).unwrap();
        let result = read_pattern_at(&bytes, TAG, pointer).unwrap().unwrap();
        assert_eq!(result.validate().unwrap(), 13);
        assert_eq!(
            result.rings.iter().map(|r| r.pellets).collect::<Vec<_>>(),
            [1, 12]
        );
        checked_pattern(Some(result)).unwrap();
        assert!(read_pattern_at(&bytes, TAG + 1, pointer).is_err());
        for changed_at in [pattern - 4, pattern + 4, pattern + 16, pattern + 0x60] {
            let mut changed = bytes.clone();
            changed[changed_at] ^= 1;
            assert!(read_pattern_at(&changed, TAG, pointer).is_err());
        }
        assert!(read_pattern_at(&bytes[..bytes.len() - 20], TAG, pointer).is_err());
        assert!(checked_pattern(Some(crate::weapon::barrel::Pattern::circular(64))).is_ok());
        assert!(checked_pattern(Some(crate::weapon::barrel::Pattern::circular(65))).is_err());
    }

    #[test]
    fn akimbo_native_variant_selects_default_and_rejects_ambiguity() {
        let mut content = crate::weapon::behavior::Content {
            owner_tag: 0x8152c7c5,
            owner: vec![0; 0x800],
            resource: 0x80,
            blocks: vec![0x180, 0x400, 0x5C0],
        };
        crate::tag_payload::write_u64(&mut content.owner, 0x88, 0x100).unwrap();
        crate::tag_payload::write_u64(&mut content.owner, 0x340, 2).unwrap();
        crate::tag_payload::write_u64(&mut content.owner, 0x348, 0xA8).unwrap();
        crate::tag_payload::write_u64(&mut content.owner, 0x3F0, 2).unwrap();
        write_u32(&mut content.owner, 0x3F8, 0x80803ACF).unwrap();
        for (index, block) in content.blocks.iter().copied().enumerate() {
            write_u32(&mut content.owner, block + 0x10, 100 + index as u32).unwrap();
            write_u32(&mut content.owner, block + 0xf0, u32::MAX).unwrap();
        }
        assert_eq!(
            selected_variant(&content, 101).unwrap(),
            SelectedVariant {
                index: 0,
                projectile: u32::MAX,
                burst_override: 0
            }
        );
        assert_eq!(
            selected_variant(&content, 102).unwrap(),
            SelectedVariant {
                index: 1,
                projectile: u32::MAX,
                burst_override: 0
            }
        );
        assert_eq!(
            selected_variant(&content, 100).unwrap(),
            SelectedVariant {
                index: u32::MAX,
                projectile: u32::MAX,
                burst_override: 0
            }
        );
        write_u32(&mut content.owner, 0x5C0 + 0x10, 101).unwrap();
        assert!(
            selected_variant(&content, 101)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        write_u32(&mut content.owner, 0x5C0 + 0x10, 102).unwrap();
        write_u32(&mut content.owner, 0x5C0 + 0xf0, 0x80bc103e).unwrap();
        assert_eq!(
            selected_variant(&content, 102).unwrap(),
            SelectedVariant {
                index: 1,
                projectile: 0x80bc103e,
                burst_override: 0
            }
        );
        for invalid in [0, 0x7FFF_FFFF, 0x811C_9DC5] {
            write_u32(&mut content.owner, 0x5C0 + 0xf0, invalid).unwrap();
            assert!(selected_variant(&content, 102).is_err());
        }
        write_u32(&mut content.owner, 0x5C0 + 0xF0, u32::MAX).unwrap();
        crate::tag_payload::write_u64(&mut content.owner, 0x5C0 + 0x110, 1).unwrap();
        assert!(selected_variant(&content, 102).is_err());
        crate::tag_payload::write_u64(&mut content.owner, 0x5C0 + 0x110, 0).unwrap();
        for burst in [0, 1, 2, 3, 64] {
            write_u32(&mut content.owner, 0x5C0 + 0x140, burst).unwrap();
            assert!(selected_variant(&content, 102).is_ok());
        }
        for burst in [65, u32::MAX] {
            write_u32(&mut content.owner, 0x5C0 + 0x140, burst).unwrap();
            assert!(
                selected_variant(&content, 102)
                    .unwrap_err()
                    .to_string()
                    .contains("Content-defined burst")
            );
        }
        write_u32(&mut content.owner, 0x5C0 + 0x140, 0).unwrap();
        crate::tag_payload::write_u64(&mut content.owner, 0x348, (-8i64) as u64).unwrap();
        assert!(
            selected_variant(&content, 102)
                .unwrap_err()
                .to_string()
                .contains("forward array")
        );
    }

    #[test]
    fn akimbo_rejects_duplicate_unbound_core_and_changed_arms_dependency() {
        let mut entity = vec![0; 0x40];
        let mut rows = vec![0; 24];
        for at in [0, 12] {
            write_u32(&mut rows, at, 0x8152C7C3).unwrap();
        }
        crate::tag_payload::append_native_array(&mut entity, 0x10, 0x80809C04, 2, &rows).unwrap();
        assert!(unique_core_owner(&entity, 0x8152C7C3).is_err());
        crate::tag_payload::append_native_array(&mut entity, 0x10, 0x80809C04, 1, &rows[..12])
            .unwrap();
        assert!(unique_core_owner(&entity, 0x8152C7C3).is_ok());
        let stock = |tag: u32| Ok(tag.to_le_bytes().to_vec());
        assert!(unchanged_arms(&[1, 2, 3], stock, stock).is_ok());
        for changed in [1, 2, 3, 0x80BDE357] {
            let pending = |tag: u32| {
                Ok(if tag == changed {
                    vec![0]
                } else {
                    tag.to_le_bytes().to_vec()
                })
            };
            assert!(unchanged_arms(&[1, 2, 3], pending, stock).is_err());
        }
    }
}
