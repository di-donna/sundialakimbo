//! Link the extended equipment and first-person rigs as one private dependency group.
use super::*;
use crate::tag_payload::read_u64;
use crate::weapon::akimbo::{deployment::Profile, visual::pose};
use parhelion_import::tiger::animation::rig;
use sundial::package_authoring::entity::{weapon_component_bindings, weapon_entity_assignment};

fn typed_reference(payload: &[u8], at: usize, target: &[u8]) -> AuthoringResult<bool> {
    if at + 16 > payload.len() {
        return Ok(false);
    }
    let class = read_u32(payload, at + 4)?;
    if class & 0xffff_0000 != 0x8080_0000 {
        return Ok(false);
    }
    let offset = usize::try_from(read_u64(payload, at + 8)?)
        .map_err(|_| invalid("Akimbo typed rig reference exceeds address width"))?;
    Ok(offset.is_multiple_of(8)
        && offset < target.len()
        && (offset >= 4 && read_u32(target, offset - 4).ok() == Some(class)
            || read_u32(target, offset + 4).ok() == Some(class)))
}

/// A marker owner can contain unrelated hashes equal to its tag. Relocate only object
/// references whose native type and interior offset resolve back into the same owner.
pub(super) fn self_references(payload: &[u8], tag: u32) -> AuthoringResult<Vec<usize>> {
    let mut fields = Vec::new();
    for at in (0..payload.len().saturating_sub(3)).step_by(4) {
        if read_u32(payload, at)? == tag && typed_reference(payload, at, payload)? {
            fields.push(at);
        }
    }
    Ok(fields)
}

/// Only validated entity owner slots, typed object references and explicit asset links
/// are relocated. Equal integers in animation data and hashed names are left intact.
pub(super) fn link_fields(
    payload: &[u8],
    entity: bool,
    names: &BTreeMap<u32, &str>,
    targets: &BTreeMap<u32, &[u8]>,
    explicit: &[(usize, u32)],
) -> AuthoringResult<BTreeMap<usize, String>> {
    let mut fields = BTreeMap::new();
    if entity {
        let (count, _, rows, class) = array(payload, 0x10)?;
        if class != 0x80809C04 || count > 256 {
            return Err(invalid("Akimbo entity component list differs"));
        }
        for index in 0..count {
            let at = rows + index * 12;
            let owner = read_u32(payload, at)?;
            if let Some(symbol) = names.get(&owner) {
                let mut relocated = payload.to_vec();
                const STAND_IN: u32 = 0x8FFF_FFFE;
                sundial::package_authoring::entity::retarget_weapon_component_owner(
                    &mut relocated,
                    owner,
                    STAND_IN,
                )
                .map_err(invalid)?;
                for at in (0..payload.len().saturating_sub(3)).step_by(4) {
                    if read_u32(payload, at)? != read_u32(&relocated, at)? {
                        if read_u32(payload, at)? != owner || read_u32(&relocated, at)? != STAND_IN
                        {
                            return Err(invalid(
                                "Akimbo entity relocation changed a non-owner word",
                            ));
                        }
                        fields.insert(at, (*symbol).to_owned());
                    }
                }
            }
        }
    }
    for at in (0..payload.len().saturating_sub(3)).step_by(4) {
        let tag = read_u32(payload, at)?;
        if let (Some(symbol), Some(target)) = (names.get(&tag), targets.get(&tag)) {
            if typed_reference(payload, at, target)? {
                fields.insert(at, (*symbol).to_owned());
            }
        }
    }
    for &(at, tag) in explicit {
        if read_u32(payload, at)? != tag {
            return Err(invalid("Akimbo explicit private graph link changed"));
        }
        let symbol = names
            .get(&tag)
            .ok_or_else(|| invalid("Akimbo private graph target is absent"))?;
        fields.insert(at, (*symbol).to_owned());
    }
    Ok(fields)
}

/// Only called for the equipment/arms skeleton whose incoming entity references
/// `rebase` already validated against the original owner and moved together.
/// A native interior interface has no tag/class prefix at the target, so the
/// general graph linker cannot recognize this proof from target bytes alone.
fn entity_rig_fields(
    payload: &[u8],
    owner: &pose::Owner,
    symbol: &str,
    fields: &mut BTreeMap<usize, String>,
) -> AuthoringResult<()> {
    const STAND_IN: u32 = 0x8FFF_FFFE;
    let mut relocated = payload.to_vec();
    sundial::package_authoring::entity::retarget_weapon_component_owner(
        &mut relocated,
        owner.tag,
        STAND_IN,
    )
    .map_err(invalid)?;
    rig::retarget_rebased_references(&mut relocated, owner.converted.len(), owner.tag, STAND_IN)
        .map_err(|error| invalid(format!("Akimbo rig owner references: {error:#}")))?;
    for at in (0..payload.len().saturating_sub(3)).step_by(4) {
        if read_u32(payload, at)? != read_u32(&relocated, at)? {
            if read_u32(payload, at)? != owner.tag || read_u32(&relocated, at)? != STAND_IN {
                return Err(invalid("Akimbo rig reference changed a non-owner word"));
            }
            fields.insert(at, symbol.to_owned());
        }
    }
    Ok(())
}

/// Nested rig instances and definitions use reciprocal references with different
/// native classes. The component retargeter proves those pairs without treating
/// arbitrary equal words (for example animation hashes) as owner references.
fn component_fields(
    payload: &[u8],
    entity: &[u8],
    owner: u32,
    symbol: &str,
    fields: &mut BTreeMap<usize, String>,
) -> AuthoringResult<()> {
    const STAND_IN: u32 = 0x8FFF_FFFE;
    let mut relocated = payload.to_vec();
    sundial::package_authoring::entity::retarget_weapon_component_owner_payload(
        &mut relocated,
        entity,
        owner,
        STAND_IN,
    )
    .map_err(invalid)?;
    for at in (0..payload.len().saturating_sub(3)).step_by(4) {
        if read_u32(payload, at)? != read_u32(&relocated, at)? {
            if read_u32(payload, at)? != owner || read_u32(&relocated, at)? != STAND_IN {
                return Err(invalid(
                    "Akimbo component relocation changed a non-owner word",
                ));
            }
            fields.insert(at, symbol.to_owned());
        }
    }
    for (&at, target) in fields.iter() {
        if target == symbol {
            write_u32(&mut relocated, at, STAND_IN)?;
        }
    }
    for at in (0..relocated.len().saturating_sub(15)).step_by(4) {
        if read_u32(&relocated, at)? == owner {
            let class = read_u32(&relocated, at + 4)?;
            let target = read_u64(&relocated, at + 8)?;
            if class & 0xFFFF_0000 == 0x8080_0000
                && target.is_multiple_of(8)
                && target
                    .checked_add(16)
                    .is_some_and(|end| end <= relocated.len() as u64)
            {
                return Err(invalid(format!(
                    "Akimbo component has an unproven owner reference at 0x{at:X}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn template(
    manager: &PackageManager,
    emission: &PackageEmission,
    tag: u32,
) -> AuthoringResult<u32> {
    let hash = TagHash(tag);
    for package in &emission.asset_packages.packages {
        if package.id == hash.pkg_id() {
            return package
                .tags
                .get(hash.entry_index() as usize)
                .map(|node| node.template_tag.0)
                .ok_or_else(|| invalid("Akimbo private rig metadata template is missing"));
        }
    }
    let allocator =
        AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
    for (index, node) in emission.private_perk_runtime_new_tags.iter().enumerate() {
        if allocator.assigned_tag(index, "Akimbo pose", "owner")? == hash {
            return Ok(node.template_tag.0);
        }
    }
    manager
        .get_entry(hash)
        .ok_or_else(|| invalid("Akimbo native rig metadata template is absent"))?;
    Ok(tag)
}

/// Marker rows refer to skeleton bones, while clip channels keep their original control slots.
pub(super) fn markers(bytes: &mut [u8], map: &rig::Renumbering) -> AuthoringResult<()> {
    let definition = relative_target(bytes, 0x18)?;
    let (count, _, rows, class) = array(bytes, definition + 0xB0)?;
    if class != 0x80808513 || count > 4096 || rows + count * 64 > bytes.len() {
        return Err(invalid("Akimbo marker set has an unsupported bone table"));
    }
    for index in 0..count {
        let at = rows + index * 64;
        // Native A1E8A0 reads availability at +0 and passes the bone at +4 to
        // B314A0/B317B0. Availability must survive the skeleton index insertion.
        let old = read_u32(bytes, at + 4)?;
        if old as usize >= map.map.len() - 1 {
            return Err(invalid("Akimbo attachment names a missing source bone"));
        }
        write_u32(bytes, at + 4, map.map[old as usize] as u32)?;
    }
    Ok(())
}

fn rebase(entity: &mut [u8], owner: &pose::Owner) -> AuthoringResult<()> {
    if let Some((cut, delta)) = owner.relocation {
        rig::rebase_entity(entity, &owner.original, owner.tag, cut, delta)
            .map_err(|error| invalid(format!("Akimbo rig interior references: {error:#}")))?;
    }
    Ok(())
}

pub(super) fn apply(
    directory: &Path,
    manager: &PackageManager,
    emission: &mut PackageEmission,
    spec: &WeaponCloneSpec,
    profile: &mut Profile,
    companions: &mut Companions,
    replacements: &mut Vec<ReplacementSpec>,
) -> AuthoringResult<pose::Prepared> {
    let pattern = sandbox_pattern_identity(&emission.sandbox_patterns, spec.identity.item_hash)
        .map_err(invalid)?
        .ok_or_else(|| invalid("Akimbo pose has no runtime pattern"))?;
    let runtime_tag =
        weapon_entity_assignment(&emission.entity_assignments, pattern.pattern_global_id_hash)
            .map_err(invalid)?
            .ok_or_else(|| invalid("Akimbo pose has no runtime entity"))?;
    let source_tags = std::cell::RefCell::new(BTreeSet::new());
    let read = |tag| {
        source_tags.borrow_mut().insert(tag);
        akimbo_profile::pending(manager, emission, tag)
    };
    let mut entity = read(runtime_tag)?;
    let arms = crate::weapon::animations::arms_rig(
        manager,
        &entity,
        Some(pattern.weapon_content_group_hash),
    )?;
    source_tags
        .borrow_mut()
        .extend(arms.source_tags.iter().copied());
    let prepared = pose::prepare(&entity, &arms.entity, &read)?;
    if profile.pose.as_ref() != Some(&prepared.pose) {
        return Err(invalid(
            "Akimbo pose changed between assessment and private allocation",
        ));
    }
    let binding = |key| -> AuthoringResult<_> {
        let bindings = weapon_component_bindings(&entity, key).map_err(invalid)?;
        match bindings.as_slice() {
            [binding] if binding.resource_count == 1 => Ok(*binding),
            _ => Err(invalid("Akimbo pose requires one attachment/marker owner")),
        }
    };
    let attachment = binding(0xD3A5500E)?;
    let marker = binding(0x04CE0B42)?;
    let mut attachment_bytes = read(attachment.owner_tag)?;
    let attachment_patch = arms.patch(arms.entity_tag);
    let arms_field = attachment.resource_offset as usize + attachment_patch.offset as usize;
    if read_u32(&attachment_bytes, arms_field)? != arms.entity_tag {
        return Err(invalid("Akimbo first-person attachment row changed"));
    }
    let mut marker_bytes = read(marker.owner_tag)?;
    let marker_original = marker_bytes.clone();
    markers(&mut marker_bytes, &prepared.renumbering)?;
    let marker_converted = marker_bytes.clone();
    let animation = read(profile.rig.animation_tag)?;
    let mut arms_entity = arms.entity.clone();
    rebase(&mut entity, &prepared.weapon_skeleton)?;
    rebase(&mut arms_entity, &prepared.arms_skeleton)?;
    // Ordinary runtime entities have no native loading-parent companion. They join the
    // existing private runtime allocation and its investment-root dependency enrollment.
    let names = BTreeMap::from([
        (runtime_tag, "akimbo-runtime"),
        (attachment.owner_tag, "akimbo-attachment"),
        (marker.owner_tag, "akimbo-markers"),
        (prepared.weapon_skeleton.tag, "akimbo-weapon-skeleton"),
        (prepared.weapon_controls.tag, "akimbo-weapon-controls"),
        (arms.entity_tag, "akimbo-arms"),
        (prepared.arms_skeleton.tag, "akimbo-arms-skeleton"),
        (prepared.arms_controls.tag, "akimbo-arms-controls"),
        (profile.rig.animation_tag, "akimbo-animation"),
    ]);
    if names.len() != 9 {
        return Err(invalid("Akimbo private rig owners overlap"));
    }
    let drafts = [
        (runtime_tag, entity),
        (attachment.owner_tag, std::mem::take(&mut attachment_bytes)),
        (marker.owner_tag, marker_bytes),
        (
            prepared.weapon_skeleton.tag,
            prepared.weapon_skeleton.converted.clone(),
        ),
        (
            prepared.weapon_controls.tag,
            prepared.weapon_controls.converted.clone(),
        ),
        (arms.entity_tag, arms_entity),
        (
            prepared.arms_skeleton.tag,
            prepared.arms_skeleton.converted.clone(),
        ),
        (
            prepared.arms_controls.tag,
            prepared.arms_controls.converted.clone(),
        ),
        (profile.rig.animation_tag, animation),
    ];
    let targets = drafts
        .iter()
        .map(|(tag, payload)| (*tag, payload.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    for (tag, payload) in &drafts {
        let mut explicit = Vec::new();
        if *tag == attachment.owner_tag {
            explicit.push((arms_field, arms.entity_tag));
        }
        if *tag == profile.rig.animation_tag {
            explicit.push((relative_target(payload, 0x10)? + 0x50, *tag));
        }
        let mut fields = link_fields(
            payload,
            *tag == runtime_tag || *tag == arms.entity_tag,
            &names,
            &targets,
            &explicit,
        )?;
        if *tag == runtime_tag {
            let owner = &prepared.weapon_skeleton;
            entity_rig_fields(payload, owner, names[&owner.tag], &mut fields)?;
        } else if *tag == arms.entity_tag {
            let owner = &prepared.arms_skeleton;
            entity_rig_fields(payload, owner, names[&owner.tag], &mut fields)?;
        }
        if *tag != runtime_tag && *tag != arms.entity_tag {
            let mut parents = Vec::new();
            for parent in [runtime_tag, arms.entity_tag] {
                let bytes = targets[&parent];
                let (count, _, rows, _) = array(bytes, 0x10)?;
                if (0..count).any(|index| read_u32(bytes, rows + index * 12).ok() == Some(*tag)) {
                    parents.push(bytes);
                }
            }
            let [parent] = parents.as_slice() else {
                return Err(invalid(
                    "Akimbo component has no unique rebased parent entity",
                ));
            };
            component_fields(payload, parent, *tag, names[tag], &mut fields)?;
        }
        let mut node = Node::new(
            names[tag],
            template(manager, emission, *tag)?,
            payload.clone(),
        );
        for (at, symbol) in fields {
            node.patch(at, symbol)?;
        }
        nodes.push(node);
    }
    let allocator =
        AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
    let base = emission.private_perk_runtime_new_tags.len();
    let symbols = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            Ok((
                node.symbol.clone(),
                allocator.assigned_tag(base + index, "Akimbo rig", "runtime resource")?,
            ))
        })
        .collect::<AuthoringResult<BTreeMap<_, _>>>()?;
    let native_graphs = [template(manager, emission, runtime_tag)?, arms.entity_tag];
    let prerequisites = sundial::package_authoring::sandbox_perk::entity::residency::additions(
        manager,
        native_graphs,
    )
    .map_err(invalid)?;
    source_tags
        .borrow_mut()
        .extend(prerequisites.iter().copied());
    let additions = symbols
        .values()
        .copied()
        .chain(prerequisites.into_iter().map(TagHash))
        .collect::<Vec<_>>();
    let loading = match &emission.runtime_dependencies {
        Some(bytes) => bytes.clone(),
        None => self::read(manager, RUNTIME_DEPENDENCY_COMPANION.0)?,
    };
    let loading = crate::shared_tag_dependency_index::enroll_dependencies(
        &loading,
        RUNTIME_DEPENDENCY_COMPANION,
        RUNTIME_DEPENDENCY_ROOT,
        &additions,
    )?;
    let mut tags = Vec::with_capacity(nodes.len());
    for mut node in nodes {
        for (at, target) in node.patches {
            if read_u32(&node.payload, at)? != u32::MAX {
                return Err(invalid("Akimbo private runtime fixup changed"));
            }
            let tag = symbols
                .get(&target)
                .ok_or_else(|| invalid("Akimbo private runtime symbol is missing"))?;
            write_u32(&mut node.payload, at, tag.0)?;
        }
        tags.push(NewTagSpec {
            template_tag: node.template,
            payload: node.payload,
            storage: node.storage,
        });
    }
    let (count, _, rows, _) = array(&emission.entity_assignments, 8)?;
    let assignments = (0..count)
        .map(|i| rows + i * 8)
        .filter(|&at| {
            read_u32(&emission.entity_assignments, at).ok() == Some(pattern.pattern_global_id_hash)
        })
        .collect::<Vec<_>>();
    let [at] = assignments.as_slice() else {
        return Err(invalid("Akimbo runtime assignment is ambiguous"));
    };
    write_u32(
        &mut emission.entity_assignments,
        at + 4,
        symbols["akimbo-runtime"].0,
    )?;
    emission.private_perk_runtime_new_tags.extend(tags);
    emission.runtime_dependencies = Some(loading);
    profile.rig.animation_tag = symbols["akimbo-animation"].0;
    profile.rig.skeleton_tag = symbols["akimbo-arms-skeleton"].0;
    profile.rig.rig_tag = symbols["akimbo-arms-controls"].0;
    emission.akimbo_source_tags.extend(source_tags.into_inner());
    let converted = |owner: &pose::Owner, kind| super::super::inspect_scene::Rig {
        native: owner.tag,
        kind,
        original: owner.original.clone(),
        converted: owner.converted.clone(),
        relocation: owner.relocation,
    };
    let rigs = [
        converted(
            &prepared.weapon_skeleton,
            super::super::inspect_scene::Kind::Skeleton,
        ),
        converted(
            &prepared.weapon_controls,
            super::super::inspect_scene::Kind::Controls,
        ),
        super::super::inspect_scene::Rig {
            native: marker.owner_tag,
            kind: super::super::inspect_scene::Kind::Markers,
            original: parhelion_import::tiger::payload::Payload(marker_original),
            converted: marker_converted,
            relocation: None,
        },
    ];
    let current_table = replacements
        .iter()
        .find(|r| r.tag == crate::weapon::crosshair::TABLE)
        .map(|r| r.payload.clone());
    if let Some(replacement) = super::super::inspect_scene::apply(
        (directory, manager),
        emission,
        (&rigs, Some(&prepared.renumbering)),
        current_table.as_deref(),
        spec.identity.item_hash,
        companions,
        true,
    )? {
        if let Some(current) = replacements.iter_mut().find(|r| r.tag == replacement.tag) {
            *current = replacement;
        } else {
            replacements.push(replacement);
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked_fixture() -> (Vec<u8>, Vec<u8>) {
        use crate::tag_payload::{append_native_array, write_u64};
        let owner = 0x81520001;
        let mut entity = vec![0; 0x80];
        let mut component = vec![0; 12];
        write_u32(&mut component, 0, owner).unwrap();
        append_native_array(&mut entity, 0x10, 0x80809C04, 1, &component).unwrap();
        let mut definition = vec![0; 8];
        write_u32(&mut definition, 0, 0x12345678).unwrap();
        write_u32(&mut definition, 4, 0x00010000).unwrap();
        append_native_array(&mut entity, 0x48, 0x80809C25, 1, &definition).unwrap();
        let mut resource = vec![0; 0x28];
        write_u32(&mut resource, 0x0C, 0x80808546).unwrap();
        write_u32(&mut resource, 0x10, owner).unwrap();
        write_u32(&mut resource, 0x14, 0x80808546).unwrap();
        write_u64(&mut resource, 0x18, 0x80).unwrap();
        append_native_array(&mut entity, 0x58, 0x80809C22, 1, &resource).unwrap();
        let mut descriptor = vec![0; 0x18];
        write_u32(&mut descriptor, 0, owner).unwrap();
        write_u32(&mut descriptor, 4, 0x80808546).unwrap();
        write_u64(&mut descriptor, 8, 0x80).unwrap();
        append_native_array(&mut entity, 0x68, 0x80809C20, 1, &descriptor).unwrap();
        let length = entity.len();
        write_u64(&mut entity, 0, length as u64).unwrap();
        let mut payload = vec![0; 0x200];
        write_u64(&mut payload, 0, 0x200).unwrap();
        for (at, class, target) in [
            (0x80, 0x80808545, 0xC0),
            (0xC0, 0x80808546, 0x80),
            (0x100, 0x80808549, 0x140),
            (0x140, 0x80808544, 0x100),
        ] {
            write_u32(&mut payload, at, owner).unwrap();
            write_u32(&mut payload, at + 4, class).unwrap();
            write_u64(&mut payload, at + 8, target).unwrap();
        }
        (entity, payload)
    }

    #[test]
    fn akimbo_rebased_interior_alias_retags_owner_without_needing_target_header() {
        use crate::tag_payload::write_u64;
        let (mut entity, original) = linked_fixture();
        let at = entity.len();
        entity.resize(at + 16, 0);
        write_u32(&mut entity, at, 0x81520001).unwrap();
        write_u32(&mut entity, at + 4, 0x808089FD).unwrap();
        write_u64(&mut entity, at + 8, 0x180).unwrap();
        let length = entity.len();
        write_u64(&mut entity, 0, length as u64).unwrap();
        let owner = pose::Owner {
            tag: 0x81520001,
            original: parhelion_import::tiger::payload::Payload(original),
            converted: vec![0; 0x280],
            relocation: Some((0x100, 0x80)),
        };
        rebase(&mut entity, &owner).unwrap();
        assert_eq!(read_u64(&entity, at + 8).unwrap(), 0x200);
        assert!(!typed_reference(&entity, at, &owner.converted).unwrap());
        let mut fields = BTreeMap::new();
        entity_rig_fields(&entity, &owner, "private", &mut fields).unwrap();
        assert_eq!(fields.get(&at).map(String::as_str), Some("private"));
        let before = entity.clone();
        for &at in fields.keys() {
            write_u32(&mut entity, at, 0x80B779E5).unwrap();
        }
        for at in (0..before.len().saturating_sub(3)).step_by(4) {
            if fields.contains_key(&at) {
                assert_eq!(read_u32(&before, at).unwrap(), owner.tag);
            } else {
                assert_eq!(before[at..at + 4], entity[at..at + 4]);
            }
        }
        assert_eq!(read_u64(&entity, at + 8).unwrap(), 0x200);
        let mut invalid_entity = before;
        write_u64(&mut invalid_entity, at + 8, 0x280).unwrap();
        assert!(
            entity_rig_fields(&invalid_entity, &owner, "private", &mut BTreeMap::new()).is_err()
        );
    }

    #[test]
    fn akimbo_component_links_include_fk_pairs_and_preserve_opaque_equal_hashes() {
        let (entity, mut payload) = linked_fixture();
        write_u32(&mut payload, 0x1E0, 0x81520001).unwrap();
        let mut fields = BTreeMap::new();
        component_fields(&payload, &entity, 0x81520001, "private", &mut fields).unwrap();
        assert_eq!(
            fields.keys().copied().collect::<Vec<_>>(),
            [0x80, 0xC0, 0x100, 0x140]
        );
        assert!(!fields.contains_key(&0x1E0));
        write_u32(&mut payload, 0xC4, 0x80808547).unwrap();
        assert!(
            component_fields(
                &payload,
                &entity,
                0x81520001,
                "private",
                &mut BTreeMap::new()
            )
            .is_err()
        );
    }

    #[test]
    #[ignore = "requires clean SUNDIAL_STOCK_PACKAGES"]
    fn akimbo_stock_pulse_rebased_rig_owners_preserve_all_non_owner_bytes() {
        let packages = crate::test_support::stock_packages();
        let manager = open_manager(&packages).unwrap();
        let mut entity = read(&manager, 0x80BB8264).unwrap();
        let mut arms = read(&manager, 0x80BC5F6D).unwrap();
        let prepared = pose::prepare(&entity, &arms, |tag| read(&manager, tag)).unwrap();
        assert_eq!(prepared.weapon_skeleton.tag, 0x80B9EAAE);
        assert_eq!(read_u32(&entity, 0x1B48).unwrap(), 0x80B9EAAE);
        assert_eq!(read_u64(&entity, 0x1B50).unwrap(), 0x298);
        rebase(&mut entity, &prepared.weapon_skeleton).unwrap();
        rebase(&mut arms, &prepared.arms_skeleton).unwrap();
        assert_eq!(read_u64(&entity, 0x1B50).unwrap(), 0x318);
        // The stale owner resolves this moved offset into the exact crashing float.
        assert_eq!(
            read_u32(&prepared.weapon_skeleton.original.0, 0x320).unwrap(),
            0x3F1BFAA2
        );
        assert_eq!(
            read_u32(&prepared.weapon_skeleton.converted, 0x320).unwrap(),
            0x80C70D35
        );
        for (parent, owner) in [
            (&entity, &prepared.weapon_skeleton),
            (&arms, &prepared.arms_skeleton),
        ] {
            let mut fields = BTreeMap::new();
            entity_rig_fields(parent, owner, "private", &mut fields).unwrap();
            let mut linked = parent.clone();
            for &at in fields.keys() {
                write_u32(&mut linked, at, 0x80B779E5).unwrap();
            }
            parhelion_import::tiger::entity::reject_stale_owner(
                &parhelion_import::tiger::payload::Payload(linked),
                owner.tag,
            )
            .unwrap();
            let mut component = BTreeMap::new();
            component_fields(
                &owner.converted,
                parent,
                owner.tag,
                "private",
                &mut component,
            )
            .unwrap();
            assert!(component.len() >= 4);
            for at in (0..owner.converted.len().saturating_sub(15)).step_by(8) {
                if read_u32(&owner.converted, at).unwrap() == owner.tag
                    && read_u32(&owner.converted, at + 4).unwrap() & 0xFFFF0000 == 0x80800000
                {
                    assert!(component.contains_key(&at), "Unlinked self owner at {at:X}");
                }
            }
            let mut copied = owner.converted.clone();
            for &at in component.keys() {
                write_u32(&mut copied, at, 0x80B779E5).unwrap();
            }
            for at in (0..copied.len().saturating_sub(3)).step_by(4) {
                if component.contains_key(&at) {
                    assert_eq!(read_u32(&owner.converted, at).unwrap(), owner.tag);
                } else {
                    assert_eq!(copied[at..at + 4], owner.converted[at..at + 4]);
                }
            }
        }
    }

    #[test]
    fn akimbo_marker_owner_links_preserve_equal_opaque_hashes() {
        let tag = 0x81520001;
        let class = 0x80808545;
        let mut payload = vec![0; 96];
        for at in [0, 16, 64] {
            write_u32(&mut payload, at, tag).unwrap();
        }
        write_u32(&mut payload, 20, class).unwrap();
        crate::tag_payload::write_u64(&mut payload, 24, 48).unwrap();
        write_u32(&mut payload, 44, class).unwrap();
        assert_eq!(self_references(&payload, tag).unwrap(), [16]);
        write_u32(&mut payload, 44, class + 1).unwrap();
        assert!(self_references(&payload, tag).unwrap().is_empty());
    }
    #[test]
    fn akimbo_marker_renumbering_preserves_availability_and_moves_only_bones() {
        let mut payload = vec![0; 0x1C0];
        crate::tag_payload::write_relative_pointer(&mut payload, 0x18, 0x100).unwrap();
        let mut rows = vec![0; 128];
        write_u32(&mut rows, 0, 19).unwrap();
        write_u32(&mut rows, 4, 3).unwrap();
        write_u32(&mut rows, 64, 3).unwrap();
        write_u32(&mut rows, 68, 0).unwrap();
        crate::tag_payload::append_native_array(&mut payload, 0x1B0, 0x80808513, 2, &rows).unwrap();
        let before = payload.clone();
        markers(&mut payload, &rig::Renumbering::inserted(5, 1, 1)).unwrap();
        let (_, _, at, _) = array(&payload, 0x1B0).unwrap();
        assert_eq!(read_u32(&payload, at).unwrap(), 19);
        assert_eq!(read_u32(&payload, at + 4).unwrap(), 4);
        assert_eq!(read_u32(&payload, at + 64).unwrap(), 3);
        assert_eq!(read_u32(&payload, at + 68).unwrap(), 0);
        let mut expected = before;
        write_u32(&mut expected, at + 4, 4).unwrap();
        assert_eq!(payload, expected);
    }
    #[test]
    fn akimbo_private_links_preserve_equal_opaque_words() {
        let tag = 0x81520001;
        let class = 0x80808545;
        let mut target = vec![0; 64];
        write_u32(&mut target, 28, class).unwrap();
        let mut payload = vec![0; 80];
        for at in [0, 16, 40, 64] {
            write_u32(&mut payload, at, tag).unwrap();
        }
        write_u32(&mut payload, 20, class).unwrap();
        crate::tag_payload::write_u64(&mut payload, 24, 32).unwrap();
        // The frozen header's size is not an object pointer; its owner is an explicit field.
        write_u32(&mut payload, 44, 0x808036E9).unwrap();
        crate::tag_payload::write_u64(&mut payload, 48, 0x3420).unwrap();
        let names = BTreeMap::from([(tag, "private")]);
        let targets = BTreeMap::from([(tag, target.as_slice())]);
        let fields = link_fields(&payload, false, &names, &targets, &[(40, tag)]).unwrap();
        assert_eq!(fields.keys().copied().collect::<Vec<_>>(), [16, 40]);
        assert_eq!(read_u32(&payload, 0).unwrap(), tag);
        assert_eq!(read_u32(&payload, 64).unwrap(), tag);
    }
}
