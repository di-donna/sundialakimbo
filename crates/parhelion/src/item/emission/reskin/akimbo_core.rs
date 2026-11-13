//! Give finite-burst controllers an identity that exists before presentation is available.
use super::*;
use crate::tag_payload::read_u64;
use crate::weapon::akimbo::deployment::Profile;
use sundial::package_authoring::entity::{
    retarget_weapon_component_owner, retarget_weapon_component_owner_payload,
    weapon_entity_assignment,
};

/// A typed-looking reference that the structural retargeter could not prove is a refusal,
/// never permission to replace every equal integer in a component payload.
fn no_remaining_reference(bytes: &[u8], old: u32) -> AuthoringResult<()> {
    for at in (0..bytes.len().saturating_sub(15)).step_by(4) {
        if read_u32(bytes, at)? != old {
            continue;
        }
        let class = read_u32(bytes, at + 4)?;
        let target = read_u64(bytes, at + 8)?;
        if class & 0x8000_0000 != 0
            && class != u32::MAX
            && target % 8 == 0
            && target < bytes.len() as u64
        {
            return Err(invalid(format!(
                "Akimbo private core has an unproven self reference at 0x{at:X}"
            )));
        }
    }
    Ok(())
}

fn copy_core(entity: &[u8], source: &[u8], old: u32, new: u32) -> AuthoringResult<Vec<u8>> {
    let mut copied = source.to_vec();
    let changed =
        retarget_weapon_component_owner_payload(&mut copied, entity, old, new).map_err(invalid)?;
    if changed < 12 {
        return Err(invalid(
            "Akimbo private core is missing its six paired native components",
        ));
    }
    no_remaining_reference(&copied, old)?;
    Ok(copied)
}

pub(super) fn apply(
    manager: &PackageManager,
    emission: &mut PackageEmission,
    spec: &WeaponCloneSpec,
    profile: &mut Profile,
) -> AuthoringResult<()> {
    if profile.adapter != 11 {
        return Err(invalid(
            "Private startup identity is required only by the finite-burst adapter",
        ));
    }
    let pattern = sandbox_pattern_identity(&emission.sandbox_patterns, spec.identity.item_hash)
        .map_err(invalid)?
        .ok_or_else(|| invalid("Akimbo private core has no final pattern"))?;
    let runtime =
        weapon_entity_assignment(&emission.entity_assignments, pattern.pattern_global_id_hash)
            .map_err(invalid)?
            .ok_or_else(|| invalid("Akimbo private core has no runtime entity"))?;
    let old = profile.barrel.core_tag;
    let original_entity = akimbo_profile::pending(manager, emission, runtime)?;
    let original_core = akimbo_profile::pending(manager, emission, old)?;
    if original_core.len() != profile.barrel.core_size as usize
        || read_u32(&original_core, 0x80)? != old
        || read_u64(&original_core, 0x88)? != u64::from(profile.barrel.root_definition)
    {
        return Err(invalid(
            "Akimbo private core changed after source assessment",
        ));
    }
    let allocator =
        AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
    let base = emission.private_perk_runtime_new_tags.len();
    let core_tag = allocator.assigned_tag(base, "Akimbo burst", "private core")?;
    let entity_tag = allocator.assigned_tag(base + 1, "Akimbo burst", "private entity")?;
    let core = copy_core(&original_entity, &original_core, old, core_tag.0)?;
    let mut entity = original_entity.clone();
    retarget_weapon_component_owner(&mut entity, old, core_tag.0).map_err(invalid)?;
    for at in akimbo_pose::self_references(&original_entity, runtime)? {
        write_u32(&mut entity, at, entity_tag.0)?;
    }
    no_remaining_reference(&entity, old)?;
    no_remaining_reference(&entity, runtime)?;
    let core_template = akimbo_pose::template(manager, emission, old)?;
    let entity_template = akimbo_pose::template(manager, emission, runtime)?;
    for template in [core_template, entity_template] {
        if manager
            .get_entry(TagHash(template))
            .is_none_or(|entry| entry.file_type == 16)
        {
            return Err(invalid(
                "Akimbo private controller must not replace a scoped loading owner",
            ));
        }
    }
    let prerequisites = sundial::package_authoring::sandbox_perk::entity::residency::additions(
        manager,
        [entity_template],
    )
    .map_err(invalid)?;
    let additions = [core_tag, entity_tag]
        .into_iter()
        .chain(prerequisites.iter().copied().map(TagHash))
        .collect::<Vec<_>>();
    let loading = match &emission.runtime_dependencies {
        Some(bytes) => bytes.clone(),
        None => read(manager, RUNTIME_DEPENDENCY_COMPANION.0)?,
    };
    let loading = crate::shared_tag_dependency_index::enroll_dependencies(
        &loading,
        RUNTIME_DEPENDENCY_COMPANION,
        RUNTIME_DEPENDENCY_ROOT,
        &additions,
    )?;
    let (count, _, rows, _) = array(&emission.entity_assignments, 8)?;
    let assignments = (0..count)
        .map(|i| rows + i * 8)
        .filter(|&at| {
            read_u32(&emission.entity_assignments, at).ok() == Some(pattern.pattern_global_id_hash)
        })
        .collect::<Vec<_>>();
    let [at] = assignments.as_slice() else {
        return Err(invalid(
            "Akimbo private core runtime assignment is ambiguous",
        ));
    };
    write_u32(&mut emission.entity_assignments, at + 4, entity_tag.0)?;
    emission.private_perk_runtime_new_tags.extend([
        NewTagSpec {
            template_tag: TagHash(core_template),
            payload: core,
            storage: crate::NewTagStorageMode::InheritTemplate,
        },
        NewTagSpec {
            template_tag: TagHash(entity_template),
            payload: entity,
            storage: crate::NewTagStorageMode::InheritTemplate,
        },
    ]);
    emission.runtime_dependencies = Some(loading);
    emission.akimbo_source_tags.extend(prerequisites);
    emission.akimbo_source_tags.extend([old, runtime]);
    profile.barrel.core_tag = core_tag.0;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn akimbo_private_core_refuses_unproven_typed_links_without_rewriting_opaque_words() {
        let old = 0x81522688;
        let mut bytes = vec![0; 64];
        write_u32(&mut bytes, 0, old).unwrap();
        let before = bytes.clone();
        no_remaining_reference(&bytes, old).unwrap();
        assert_eq!(bytes, before);
        for class in [0x80803889, 0x80BFDFEA, 0x80BFDFEB] {
            write_u32(&mut bytes, 4, class).unwrap();
            assert!(no_remaining_reference(&bytes, old).is_err());
        }
    }

    #[test]
    #[ignore = "requires clean SUNDIAL_STOCK_PACKAGES"]
    fn akimbo_stock_finite_core_clone_preserves_all_non_owner_bytes() {
        let packages = crate::test_support::stock_packages();
        let manager = open_manager(&packages).unwrap();
        let entity = read(&manager, 0x80BB8264).unwrap();
        let source = read(&manager, 0x81522688).unwrap();
        let copied = copy_core(&entity, &source, 0x81522688, 0x80B77950).unwrap();
        let mut restored = copied.clone();
        for at in (0..source.len().saturating_sub(3)).step_by(4) {
            if source[at..at + 4] != copied[at..at + 4] {
                assert_eq!(read_u32(&source, at).unwrap(), 0x81522688);
                assert_eq!(read_u32(&copied, at).unwrap(), 0x80B77950);
                restored[at..at + 4].copy_from_slice(&source[at..at + 4]);
            }
        }
        assert_eq!(source, restored);
        let definition = read_u64(&source, 0x88).unwrap() as usize;
        assert_eq!(read_u32(&copied, 0x80).unwrap(), 0x80B77950);
        assert_eq!(read_u32(&copied, definition).unwrap(), 0x80B77950);
        assert_eq!(read_u32(&source, 0x84).unwrap(), 0x80BFDFEB);
        assert_eq!(read_u32(&source, definition + 4).unwrap(), 0x80BFDFEA);
    }
}
