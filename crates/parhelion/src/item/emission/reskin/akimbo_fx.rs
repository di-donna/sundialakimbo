//! Give generated weapons private firing sequences when the native owner is reserved.
//!
//! Legacy Dawn adapters retain their native sequence tags. A generated weapon clones the
//! selected route, including its content and entity bindings, without altering stock assets
//! or the other idle/reload slots in a sequence override list.
use super::*;
use crate::weapon::akimbo::{content, deployment::Profile, visual};
use sundial::package_authoring::entity::{
    retarget_weapon_component_owner, weapon_entity_assignment,
};

pub(super) fn reserved(profile: &Profile) -> bool {
    profile.visual.as_ref().is_some_and(|visual| {
        matches!(
            visual.fx.tag,
            0x80ef262f | 0x80ff0531 | 0x80ef2608 | 0x80ef42ba | 0x80ef42e3
        )
    })
}

pub(super) fn apply(
    directory: &Path,
    manager: &PackageManager,
    emission: &mut PackageEmission,
    spec: &WeaponCloneSpec,
    profile: &mut Profile,
    companions: &mut Companions,
) -> AuthoringResult<()> {
    if !reserved(profile) {
        return Err(invalid(
            "Akimbo private firing route needs a reserved native owner",
        ));
    }
    let fx = &profile.visual.as_ref().unwrap().fx;
    let pattern = sandbox_pattern_identity(&emission.sandbox_patterns, spec.identity.item_hash)
        .map_err(invalid)?
        .ok_or_else(|| invalid("Akimbo private firing route has no runtime pattern"))?;
    let runtime =
        weapon_entity_assignment(&emission.entity_assignments, pattern.pattern_global_id_hash)
            .map_err(invalid)?
            .ok_or_else(|| invalid("Akimbo private firing route has no runtime entity"))?;
    let sources = std::cell::RefCell::new(BTreeSet::new());
    let read = |tag| {
        sources.borrow_mut().insert(tag);
        akimbo_profile::pending(manager, emission, tag)
    };
    let entity = read(runtime)?;
    let selected_content = crate::weapon::behavior::content_with(&entity, &read)?;
    if selected_content.owner_tag != profile.barrel.content_tag {
        return Err(invalid(
            "Akimbo private firing content differs from its assessed owner",
        ));
    }
    let block = content::select(&selected_content, pattern.weapon_content_group_hash)?.block;
    let direct = read_u32(&selected_content.owner, block + 0x60)?;
    let override_tag = read_u32(&selected_content.owner, block + 0xC0)?;
    let firing = visual::firing_entity(direct, override_tag, &read)?;
    let firing_entity = read(firing)?;
    let sequence = read(fx.tag)?;
    if sequence.len() != fx.bytes as usize
        || relative_target(&sequence, 0x18)? != fx.definition as usize
    {
        return Err(invalid(
            "Akimbo reserved firing owner changed after assessment",
        ));
    }
    // These drafts keep all native offsets and sizes. Only typed asset references and
    // the selected firing-route fields are relocated by the private linker below.
    let mut drafts = vec![
        (runtime, "akimbo-fx-runtime", entity),
        (
            selected_content.owner_tag,
            "akimbo-fx-content",
            selected_content.owner,
        ),
        (firing, "akimbo-fx-entity", firing_entity),
        (fx.tag, "akimbo-fx-sequence", sequence),
    ];
    let mut explicit = BTreeMap::<u32, Vec<(usize, u32)>>::new();
    if override_tag == u32::MAX {
        explicit.insert(selected_content.owner_tag, vec![(block + 0x60, firing)]);
    } else {
        // firing_entity already checked the wrapper, all six slots, and their names.
        let wrapper = read(override_tag)?;
        let list = read_u32(&wrapper, 16)?;
        drafts.push((override_tag, "akimbo-fx-wrapper", wrapper));
        drafts.push((list, "akimbo-fx-overrides", read(list)?));
        explicit.insert(
            selected_content.owner_tag,
            vec![(block + 0xC0, override_tag)],
        );
        explicit.insert(override_tag, vec![(16, list)]);
        explicit.insert(list, vec![(24, firing)]);
    }
    let names = drafts
        .iter()
        .map(|(tag, name, _)| (*tag, *name))
        .collect::<BTreeMap<_, _>>();
    if names.len() != drafts.len() {
        return Err(invalid(
            "Akimbo private firing graph has overlapping owners",
        ));
    }
    // Prove both expected component links exist before using the shared typed linker.
    for (entity, owner) in [
        (&drafts[0].2, selected_content.owner_tag),
        (&drafts[2].2, fx.tag),
    ] {
        let mut relocated = entity.clone();
        retarget_weapon_component_owner(&mut relocated, owner, 0x8fff_fffe).map_err(invalid)?;
        if relocated == *entity {
            return Err(invalid(
                "Akimbo private firing entity is missing its expected owner",
            ));
        }
    }
    let targets = drafts
        .iter()
        .map(|(tag, _, bytes)| (*tag, bytes.as_slice()))
        .collect();
    let mut nodes = Vec::new();
    for (tag, name, payload) in &drafts {
        let fields = akimbo_pose::link_fields(
            payload,
            *tag == runtime || *tag == firing,
            &names,
            &targets,
            explicit.get(tag).map(Vec::as_slice).unwrap_or(&[]),
        )?;
        let mut node = Node::new(
            *name,
            akimbo_pose::template(manager, emission, *tag)?,
            payload.clone(),
        );
        for (at, symbol) in fields {
            node.patch(at, symbol)?;
        }
        nodes.push(node);
    }
    let prerequisites = sundial::package_authoring::sandbox_perk::entity::residency::additions(
        manager,
        [
            akimbo_pose::template(manager, emission, runtime)?,
            akimbo_pose::template(manager, emission, firing)?,
        ],
    )
    .map_err(invalid)?;
    sources.borrow_mut().extend(prerequisites.iter().copied());
    let source_tags = sources.into_inner();
    let (symbols, enrollment) = allocate(directory, manager, emission, nodes, companions)?;
    let additions = enrollment
        .into_iter()
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
    let (count, _, rows, _) = array(&emission.entity_assignments, 8)?;
    let assignments = (0..count)
        .map(|index| rows + index * 8)
        .filter(|&at| {
            read_u32(&emission.entity_assignments, at).ok() == Some(pattern.pattern_global_id_hash)
        })
        .collect::<Vec<_>>();
    let [at] = assignments.as_slice() else {
        return Err(invalid(
            "Akimbo private firing runtime assignment is ambiguous",
        ));
    };
    write_u32(
        &mut emission.entity_assignments,
        at + 4,
        symbols["akimbo-fx-runtime"].0,
    )?;
    emission.runtime_dependencies = Some(loading);
    emission.akimbo_source_tags.extend(source_tags);
    profile.barrel.content_tag = symbols["akimbo-fx-content"].0;
    // Reassessment must follow the final entity/content route before publication.
    profile.visual = None;
    Ok(())
}

fn allocate(
    directory: &Path,
    manager: &PackageManager,
    emission: &mut PackageEmission,
    nodes: Vec<Node>,
    companions: &mut Companions,
) -> AuthoringResult<(BTreeMap<String, TagHash>, Vec<TagHash>)> {
    let mut loading = Vec::new();
    for node in &nodes {
        let entry = manager
            .get_entry(node.template)
            .ok_or_else(|| invalid("Akimbo firing metadata template is missing"))?;
        if entry.file_type == 16 {
            if entry.file_subtype != 0 {
                return Err(invalid(
                    "Akimbo firing loading owner has an unsupported subtype",
                ));
            }
            let mut companion = Node::new(format!("{}-companion", node.symbol), 0, Vec::new());
            companion.companion = Some(Companion::new(node.symbol.clone(), node.template.0));
            loading.push(companion);
        }
    }
    if let Some(first) = loading.first() {
        let primary = first.companion.as_ref().unwrap().shared_owner.clone();
        // The weapon entity and content are globally resident. The native override
        // wrapper owns only its firing graph, loaded through the selected content
        // field. Putting the weapon itself under that wrapper creates a loading cycle.
        let (global, mut scoped): (Vec<_>, Vec<_>) = nodes.into_iter().partition(|node| {
            matches!(
                node.symbol.as_str(),
                "akimbo-fx-runtime" | "akimbo-fx-content"
            )
        });
        if global.len() != 2
            || scoped.iter().any(|node| {
                node.patches
                    .iter()
                    .any(|(_, target)| global.iter().any(|global| global.symbol == *target))
            })
        {
            return Err(invalid(
                "Akimbo firing graph crosses its native loading boundary",
            ));
        }
        let allocator =
            AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
        let base = emission.private_perk_runtime_new_tags.len();
        let mut symbols = global
            .iter()
            .enumerate()
            .map(|(index, node)| {
                Ok((
                    node.symbol.clone(),
                    allocator.assigned_tag(base + index, "Akimbo firing", "runtime resource")?,
                ))
            })
            .collect::<AuthoringResult<BTreeMap<_, _>>>()?;
        let enrollment = symbols.values().copied().collect();
        scoped.extend(loading);
        let linked = linking::link(
            directory,
            emission,
            manager,
            scoped,
            &primary,
            companions,
            0,
            |_, _| Ok(()),
            None,
        )?;
        symbols.extend(linked.symbols);
        append_runtime(emission, global, &symbols)?;
        return Ok((symbols, enrollment));
    }
    // A direct sequence route without a native loading parent joins the same global
    // runtime allocation as the ordinary weapon/arms graph. Never invent a companion.
    let allocator =
        AppendedTagAllocator::private_runtime(emission.private_perk_runtime_append_start);
    let base = emission.private_perk_runtime_new_tags.len();
    let symbols = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            Ok((
                node.symbol.clone(),
                allocator.assigned_tag(base + index, "Akimbo firing", "runtime resource")?,
            ))
        })
        .collect::<AuthoringResult<BTreeMap<_, _>>>()?;
    append_runtime(emission, nodes, &symbols)?;
    let enrollment = symbols.values().copied().collect();
    Ok((symbols, enrollment))
}

fn append_runtime(
    emission: &mut PackageEmission,
    nodes: Vec<Node>,
    symbols: &BTreeMap<String, TagHash>,
) -> AuthoringResult<()> {
    let mut tags = Vec::new();
    for mut node in nodes {
        for (at, symbol) in node.patches {
            if read_u32(&node.payload, at)? != u32::MAX {
                return Err(invalid("Akimbo private firing fixup changed"));
            }
            write_u32(&mut node.payload, at, symbols[&symbol].0)?;
        }
        tags.push(NewTagSpec {
            template_tag: node.template,
            payload: node.payload,
            storage: node.storage,
        });
    }
    emission.private_perk_runtime_new_tags.extend(tags);
    Ok(())
}
