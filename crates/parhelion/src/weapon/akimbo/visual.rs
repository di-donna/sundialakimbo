//! Derive native visual bindings from the final weapon graph and appearance.
//!
//! This follows typed component, event, selector and marker arrays. No weapon ID, particle
//! program, attachment transform or selector offset is borrowed from another donor.
use std::collections::{BTreeMap, BTreeSet};

use super::deployment::{Fx, FxSelector, MarkerBone, Visual};
use crate::tag_payload::{array_at, read_u32, read_u64, relative_target};
use crate::{AuthoringResult, error::invalid};
use sundial::package_authoring::entity::{WEAPON_BARREL_COMPONENT_KEY, weapon_component_bindings};

const LIMIT: usize = 16 * 1024 * 1024;
const MUZZLE: u32 = 0x60FE_CDD4;
const EJECT: u32 = 0x9216_EEA9;
const MARKER_BINDING: u32 = 0x04CE_0B42;
type Read<'a> = dyn Fn(u32) -> AuthoringResult<Vec<u8>> + 'a;

pub(crate) mod pose;
mod rig;

fn bounded(read: &Read<'_>, tag: u32) -> AuthoringResult<Vec<u8>> {
    if tag == 0 || tag == u32::MAX {
        return Err(invalid("Akimbo visual graph names an absent asset"));
    }
    let bytes = read(tag)?;
    if bytes.is_empty() || bytes.len() > LIMIT {
        return Err(invalid("Akimbo visual asset exceeds the supported size"));
    }
    Ok(bytes)
}

fn array(
    bytes: &[u8],
    at: usize,
    class: u32,
    stride: usize,
    maximum: usize,
) -> AuthoringResult<(usize, usize)> {
    let (count, header, rows, actual) = array_at(bytes, at)?;
    let typed = header
        .checked_sub(4)
        .is_some_and(|before| read_u32(bytes, before).ok() == Some(0x8080_9FBD))
        && read_u64(bytes, header + 8)? == u64::from(class);
    if count > maximum
        || (count != 0 && (actual != class || !typed))
        || rows
            .checked_add(count.saturating_mul(stride))
            .is_none_or(|end| end > bytes.len())
    {
        return Err(invalid(format!(
            "Akimbo visual array at 0x{at:X} has an unsupported layout"
        )));
    }
    Ok((count, rows))
}

struct Component {
    tag: u32,
    bytes: Vec<u8>,
    header: usize,
    definition: usize,
    class: u32,
}

fn component(tag: u32, read: &Read<'_>) -> AuthoringResult<Component> {
    let bytes = bounded(read, tag)?;
    let header = relative_target(&bytes, 0x10)?;
    let definition = relative_target(&bytes, 0x18)?;
    if header.checked_add(16).is_none_or(|end| end > bytes.len()) || definition >= bytes.len() {
        return Err(invalid(
            "Akimbo visual component header is outside its asset",
        ));
    }
    let class = read_u32(&bytes, header + 4)?;
    if read_u64(&bytes, 0)? != bytes.len() as u64
        || read_u32(&bytes, header)? != tag
        || read_u64(&bytes, header + 8)? != definition as u64
        || definition >= bytes.len()
    {
        return Err(invalid(
            "Akimbo visual component has inconsistent native ownership",
        ));
    }
    Ok(Component {
        tag,
        bytes,
        header,
        definition,
        class,
    })
}

fn components(entity: &[u8]) -> AuthoringResult<Vec<u32>> {
    let (count, rows) = array(entity, 0x10, 0x8080_9C04, 12, 256)?;
    (0..count)
        .map(|index| read_u32(entity, rows + index * 12))
        .collect()
}

fn one_component(entity: &[u8], class: u32, read: &Read<'_>) -> AuthoringResult<Component> {
    let mut found = None;
    for tag in components(entity)? {
        let current = component(tag, read)?;
        if current.class == class && found.replace(current).is_some() {
            return Err(invalid(format!(
                "Akimbo visual component 0x{class:08X} is ambiguous"
            )));
        }
    }
    found.ok_or_else(|| invalid(format!("Akimbo visual component 0x{class:08X} is absent")))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Marker {
    availability: u32,
    bone: u32,
    bits: [u32; 8],
}

fn markers(
    owner: &Component,
    marker_bone: u32,
    additional: &[u32],
) -> AuthoringResult<BTreeMap<u32, Vec<Marker>>> {
    if owner.class != 0x8080_8507
        || owner.header < 4
        || read_u32(&owner.bytes, owner.header - 4)? != 0x8080_8506
    {
        return Err(invalid(
            "Akimbo attachment owner is not a supported marker component",
        ));
    }
    let (count, rows) = array(&owner.bytes, owner.definition + 0xB0, 0x8080_8513, 64, 4096)?;
    let mut result = BTreeMap::<u32, Vec<Marker>>::new();
    let mut previous_name = 0;
    for index in 0..count {
        let row = rows + index * 64;
        let availability = read_u32(&owner.bytes, row)?;
        let bone = read_u32(&owner.bytes, row + 4)?;
        if bone == marker_bone {
            return Err(invalid(
                "Akimbo pose marker bone is used by a native attachment",
            ));
        }
        let name = read_u32(&owner.bytes, row + 0x30)?;
        if index != 0 && name < previous_name {
            return Err(invalid(
                "Akimbo attachment names are not in native lookup order",
            ));
        }
        previous_name = name;
        if name != MUZZLE && name != EJECT && !additional.contains(&name) {
            continue;
        }
        let mut bits = [0; 8];
        for (index, word) in bits.iter_mut().enumerate() {
            *word = read_u32(&owner.bytes, row + 0x10 + index * 4)?;
        }
        result.entry(name).or_default().push(Marker {
            availability,
            bone,
            bits,
        });
    }
    Ok(result)
}

fn frame(rows: Option<&Vec<Marker>>, name: &str) -> AuthoringResult<[u32; 8]> {
    local_frame(rows, name, 1)
}

fn local_frame(rows: Option<&Vec<Marker>>, name: &str, bones: usize) -> AuthoringResult<[u32; 8]> {
    let Some((marker, remaining)) = rows.and_then(|rows| rows.split_first()) else {
        return Err(invalid(format!("Akimbo has no {name} marker")));
    };
    // A1F640 walks equal-name rows in source order and stops at the selector's
    // capacity. A1E8A0 tests row+0 with 599000 before using bone row+4. When
    // every candidate has the same availability index they are enabled together:
    // a populated capacity-one selector therefore always uses the first row.
    if remaining
        .iter()
        .any(|other| other.availability != marker.availability || other.bone as usize >= bones)
    {
        return Err(invalid(format!(
            "Akimbo {name} markers have independently changing visibility"
        )));
    }
    let values = marker.bits.map(f32::from_bits);
    let norm: f32 = values[..4].iter().map(|x| x * x).sum();
    if marker.bone as usize >= bones {
        return Err(invalid(format!(
            "Akimbo {name} marker bone {} is outside the {bones}-bone weapon palette",
            marker.bone
        )));
    }
    if values.iter().any(|x| !x.is_finite())
        || norm <= 0.96
        || norm >= 1.04
        || values[4..7].iter().any(|x| x.abs() >= 3.0)
        || values[7] <= 0.5
        || values[7] >= 2.0
    {
        return Err(invalid(format!(
            "Akimbo {name} marker frame is outside the checked transform range: {values:?}"
        )));
    }
    Ok(marker.bits)
}

fn marker_frame(
    rows: Option<&Vec<Marker>>,
    name: &str,
    role: u32,
    index: u32,
    binds: &[[u32; 8]],
    bindings: &mut Vec<MarkerBone>,
) -> AuthoringResult<[u32; 8]> {
    let bits = local_frame(rows, name, binds.len())?;
    let bone = rows.unwrap()[0].bone;
    if bone != 0 {
        bindings.push(MarkerBone {
            role,
            index,
            bone,
            bind_bits: binds[bone as usize],
        });
    }
    Ok(bits)
}

fn attachment_frames(
    entity: &[u8],
    gear_entities: &[u32],
    pose: Option<&super::deployment::GunPose>,
    requires_ejection: bool,
    additional: &[u32],
    read: &Read<'_>,
) -> AuthoringResult<([u32; 8], [u32; 8], Vec<Vec<[u32; 8]>>, Vec<MarkerBone>)> {
    let marker_bone = pose.map_or(3, |pose| pose.marker_bone);
    let identity = [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()];
    let binds = if let Some(pose) = pose {
        let skeleton = rig::bound(entity, 0x1C80DD4A, 0x80808545, read)?;
        let (count, rows) = array(
            &skeleton.bytes,
            skeleton.definition + 0x90,
            0x80809F75,
            32,
            56,
        )?;
        if count != pose.weapon_bone_count as usize {
            return Err(invalid(
                "Akimbo attachment palette differs from its private rig",
            ));
        }
        let frames = (0..count)
            .map(|index| {
                rig::frame_at(&skeleton.bytes, rows + index * 32)
                    .map(|frame| frame.map(f32::to_bits))
            })
            .collect::<AuthoringResult<Vec<_>>>()?;
        if !rig::same_frame(frames[0].map(f32::from_bits), identity.map(f32::from_bits)) {
            return Err(invalid("Akimbo attachment palette root is not identity"));
        }
        frames
    } else {
        vec![identity]
    };
    let bindings = weapon_component_bindings(entity, MARKER_BINDING).map_err(invalid)?;
    let [binding] = bindings.as_slice() else {
        return Err(invalid("Akimbo needs one native marker-set binding"));
    };
    let owner = component(binding.owner_tag, read)?;
    if binding.resource_count != 1
        || binding.resource_offset != owner.header as u64
        // Entity bindings name the marker interface; its in-memory instance is 80808507.
        || binding.concrete_class != 0x8080_8506
    {
        return Err(invalid(
            "Akimbo marker-set binding differs from its component",
        ));
    }
    let mut selected = markers(&owner, marker_bone, additional)?;
    let mut overrides = BTreeMap::<u32, Vec<Marker>>::new();
    let mut visited = BTreeSet::new();
    if gear_entities.is_empty() || gear_entities.len() > 256 {
        return Err(invalid(
            "Akimbo visual assessment needs the final appearance entities",
        ));
    }
    for &tag in gear_entities {
        for tag in components(&bounded(read, tag)?)? {
            if !visited.insert(tag) {
                continue;
            }
            let owner = component(tag, read)?;
            if owner.class != 0x8080_8507 {
                continue;
            }
            for (name, rows) in markers(&owner, marker_bone, additional)? {
                if let Some(previous) = overrides.insert(name, rows.clone())
                    && previous != rows
                {
                    return Err(invalid(
                        "Akimbo appearance has conflicting attachment markers",
                    ));
                }
            }
        }
    }
    selected.extend(overrides);
    let mut marker_bones = Vec::new();
    let muzzle = marker_frame(
        selected.get(&MUZZLE),
        "muzzle",
        1,
        0,
        &binds,
        &mut marker_bones,
    )?;
    let eject = if !requires_ejection && !selected.contains_key(&EJECT) {
        ejection_frame(None, false)?
    } else {
        marker_frame(
            selected.get(&EJECT),
            "ejection",
            2,
            0,
            &binds,
            &mut marker_bones,
        )?
    };
    let additional_markers = additional
        .iter()
        .enumerate()
        .map(|(role, name)| {
            let group = selected.get(name).ok_or_else(|| {
                invalid(format!("Akimbo local effect marker 0x{name:08X} is absent"))
            })?;
            if group.is_empty() || group.len() > 6 {
                return Err(invalid("Akimbo local effect exceeds six source markers"));
            }
            group
                .iter()
                .enumerate()
                .map(|(index, marker)| {
                    marker_frame(
                        Some(&vec![marker.clone()]),
                        "local effect",
                        role as u32 + 3,
                        index as u32,
                        &binds,
                        &mut marker_bones,
                    )
                })
                .collect()
        })
        .collect::<AuthoringResult<_>>()?;
    Ok((muzzle, eject, additional_markers, marker_bones))
}

fn ejection_frame(markers: Option<&Vec<Marker>>, required: bool) -> AuthoringResult<[u32; 8]> {
    if markers.is_none() && !required {
        // No selector can consume this frame. Keep the descriptor's fixed frame layout
        // without inventing a casing emitter for launchers or other caseless weapons.
        return Ok([0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()]);
    }
    frame(markers, "ejection")
}

/// Content +C0 names an 8080744A wrapper around six 80803ADC sequence overrides.
/// Its first entry replaces the firing entity. It does not by itself denote a beam
/// or a continuously firing weapon; ordinary rifles also use this native route.
pub(crate) fn firing_entity(
    direct: u32,
    override_tag: u32,
    read: &Read<'_>,
) -> AuthoringResult<u32> {
    if override_tag == u32::MAX {
        return Ok(direct);
    }
    if direct != u32::MAX {
        return Err(invalid(
            "Akimbo firing content has competing direct and override routes",
        ));
    }
    let wrapper = bounded(read, override_tag)?;
    if wrapper.len() != 24
        || read_u64(&wrapper, 0)? != 24
        || read_u64(&wrapper, 8)? != 0
        || read_u32(&wrapper, 20)? != 0
    {
        return Err(invalid(
            "Akimbo firing override wrapper has an unsupported layout",
        ));
    }
    let overrides = bounded(read, read_u32(&wrapper, 16)?)?;
    if overrides.len() < 112
        || read_u64(&overrides, 0)? != overrides.len() as u64
        || read_u64(&overrides, 8)? != 0x811C_9DC5
    {
        return Err(invalid(
            "Akimbo firing sequence override list has an unsupported layout",
        ));
    }
    // Each slot is an optional source name and an entity tag with zero padding.
    // Validate all six records, then follow only the native firing slot. Empty,
    // idle, reload and other sequences retain their original engine ownership.
    for at in (16..112).step_by(16) {
        if read_u32(&overrides, at + 12)? != 0 {
            return Err(invalid(
                "Akimbo firing override entity has unexpected padding",
            ));
        }
        if read_u64(&overrides, at)? != 0 {
            let name = relative_target(&overrides, at)?;
            if name < 112 || name >= overrides.len() || !overrides[name..].contains(&0) {
                return Err(invalid(
                    "Akimbo firing override source name escapes its asset",
                ));
            }
        }
    }
    let entity = read_u32(&overrides, 24)?;
    if entity < 0x8000_0000 || matches!(entity, u32::MAX | 0x811C_9DC5) {
        return Err(invalid("Akimbo sequence override has no firing entity"));
    }
    Ok(entity)
}

fn tracer(entity: &[u8], projectile_override: u32, read: &Read<'_>) -> AuthoringResult<(u32, u32)> {
    // The selected weapon-content block overrides the barrel's fallback projectile.
    // Resolve that exact graph; accepting it still requires the same unique, typed
    // movement owner and supported native layout as the fallback route.
    let projectile = if projectile_override == u32::MAX {
        barrel_projectile(entity, read)?
    } else {
        projectile_override
    };
    tracer_owner(projectile, read)
}

fn barrel_projectile(entity: &[u8], read: &Read<'_>) -> AuthoringResult<u32> {
    let bindings =
        weapon_component_bindings(entity, WEAPON_BARREL_COMPONENT_KEY).map_err(invalid)?;
    let [binding] = bindings.as_slice() else {
        return Err(invalid("Akimbo visual assessment needs one native barrel"));
    };
    let owner = bounded(read, binding.owner_tag)?;
    let slots = sundial::package_authoring::entity::barrel::projectile_slots(&owner, *binding)
        .map_err(invalid)?;
    let [(_, Some(first)), (_, Some(second))] = slots.as_slice() else {
        return Err(invalid("Akimbo barrel needs one native projectile route"));
    };
    if first != second {
        return Err(invalid(
            "Akimbo barrel names different native projectile routes",
        ));
    }
    Ok(*first)
}

fn tracer_owner(projectile: u32, read: &Read<'_>) -> AuthoringResult<(u32, u32)> {
    let owner = one_component(&bounded(read, projectile)?, 0x8080_388F, read)?;
    if owner.header != 0x130 || !owner.definition.is_multiple_of(8) {
        return Err(invalid(
            "Akimbo tracer uses an unsupported movement-component layout",
        ));
    }
    Ok((owner.tag, owner.definition as u32))
}

#[derive(Clone, Copy)]
struct Binding {
    role: u32,
    capacity: u32,
}

fn effect_bindings(owner: &Component) -> AuthoringResult<(Vec<Binding>, u32, Vec<u32>)> {
    let (count, rows) = array(&owner.bytes, owner.definition + 0x1C0, 0x8080_8615, 16, 256)?;
    let mut result = Vec::with_capacity(count);
    let mut endpoint = None;
    let mut additional = Vec::new();
    for index in 0..count {
        let at = rows + index * 16;
        let kind = owner.bytes[at];
        let mask = read_u32(&owner.bytes, at + 4)?;
        let name = read_u32(&owner.bytes, at + 8)?;
        if owner.bytes[at + 3] != 0 || read_u32(&owner.bytes, at + 12)? != 0xFF {
            return Err(invalid(
                "Akimbo firing attachment has an unsupported binding layout",
            ));
        }
        let role = if kind == 3 && mask == 0x22 {
            match name {
                MUZZLE => 1,
                EJECT => 2,
                _ => {
                    let index = if let Some(index) =
                        additional.iter().position(|value| *value == name)
                    {
                        index
                    } else {
                        if additional.len() == 14 {
                            return Err(invalid(
                                "Akimbo firing effects exceed fourteen additional attachment groups",
                            ));
                        }
                        additional.push(name);
                        additional.len() - 1
                    };
                    3 + index as u32
                }
            }
        } else {
            0
        };
        if kind == 1 && mask == 0x5F && endpoint.replace(index as u32).is_some() {
            return Err(invalid(
                "Akimbo firing effects have ambiguous endpoint bindings",
            ));
        }
        result.push(Binding {
            role,
            capacity: u32::from(owner.bytes[at + 2]),
        });
    }
    Ok((
        result,
        endpoint.ok_or_else(|| invalid("Akimbo firing effects have no endpoint binding"))?,
        additional,
    ))
}

fn selector(
    owner: &Component,
    at: usize,
    bindings: &[Binding],
    endpoint: u32,
    read: &Read<'_>,
) -> AuthoringResult<Option<FxSelector>> {
    let (inputs, rows) = array(&owner.bytes, at, 0x8080_0009, 1, 4)?;
    if inputs == 0 {
        // Attachment-free particles have no weapon-local frame to relocate.
        return Ok(None);
    }
    let index = usize::from(owner.bytes[rows]);
    let binding = bindings
        .get(index)
        .ok_or_else(|| invalid("Akimbo particle selector names an absent binding"))?;
    // Every input must refer to an existing binding, including untouched world-space effects.
    let inputs_valid = owner.bytes[rows..rows + inputs]
        .iter()
        .all(|&b| usize::from(b) < bindings.len());
    if !inputs_valid {
        return Err(invalid(
            "Akimbo particle selector has an out-of-range attachment",
        ));
    }
    if binding.role == 0 {
        if owner.bytes[rows..rows + inputs]
            .iter()
            .any(|&b| bindings[usize::from(b)].role != 0)
        {
            return Err(invalid(
                "Akimbo local particle attachment occurs in an unsupported input slot",
            ));
        }
        return Ok(None);
    }
    if !matches!(inputs, 1 | 2)
        || !(1..=6).contains(&binding.capacity)
        || (binding.role <= 2 && binding.capacity != 1)
        || index as u32 == endpoint
        || (inputs == 2 && u32::from(owner.bytes[rows + 1]) != endpoint)
    {
        return Err(invalid(
            "Akimbo local particles need one marker and at most one endpoint input",
        ));
    }
    let particle = read_u32(&owner.bytes, at + 16)?;
    let program = read_u32(&bounded(read, particle)?, 0)?;
    let bytes = bounded(read, program)?;
    if read_u64(&bytes, 0)? != bytes.len() as u64
        || read_u64(&bytes, 0x18)? != 2
        || bytes.get(0x38..0x40) != Some(&[0, 1, 1, 1, 0, 0, 0, 0])
    {
        return Err(invalid(
            "Akimbo local particle program has an unsupported attachment mapping",
        ));
    }
    let groups = u32::from(
        *bytes
            .get(0x47)
            .ok_or_else(|| invalid("Akimbo particle program is truncated"))?,
    );
    if groups < inputs as u32 || groups > 3 {
        return Err(invalid(
            "Akimbo local particle program has an unsupported group count",
        ));
    }
    Ok(Some(FxSelector {
        offset: at as u32,
        particle,
        binding: index as u32,
        inputs: inputs as u32,
        markers: binding.capacity,
        groups,
        program,
        program_bytes: bytes.len() as u32,
        role: binding.role,
    }))
}

fn effects(entity_tag: u32, read: &Read<'_>) -> AuthoringResult<(Fx, Vec<u32>)> {
    let owner = one_component(&bounded(read, entity_tag)?, 0x8080_84E9, read)?;
    if owner.header != 0x90 || !owner.definition.is_multiple_of(8) {
        return Err(invalid(
            "Akimbo firing sequence has an unsupported native header",
        ));
    }
    let (bindings, endpoint, additional) = effect_bindings(&owner)?;
    let (count, rows) = array(
        &owner.bytes,
        owner.definition + 0x168,
        0x8080_93E6,
        24,
        4096,
    )?;
    let mut selectors = BTreeMap::new();
    for index in 0..count {
        let event = relative_target(&owner.bytes, rows + index * 24 + 16)?;
        let class = event
            .checked_sub(4)
            .ok_or_else(|| invalid("Akimbo firing event has no class"))?;
        match read_u32(&owner.bytes, class)? {
            // Native sound and camera events have no particle selector to relocate.
            0x8080_6F49 | 0x8080_3D8D => continue,
            // Native sequence control events carry a resource and scalar argument lists,
            // not particle attachment selectors. Preserve the native event once.
            0x8080_6B38 => {
                if event
                    .checked_add(120)
                    .is_none_or(|end| end > owner.bytes.len())
                    || read_u32(&owner.bytes, event)? != owner.tag
                    || read_u32(&owner.bytes, event + 4)? != 0x8080_6B37
                {
                    return Err(invalid(
                        "Akimbo sequence control event has inconsistent ownership",
                    ));
                }
                continue;
            }
            0x8080_6CC6 => {}
            other => {
                return Err(invalid(format!(
                    "Akimbo firing event 0x{other:08X} needs a visual adapter"
                )));
            }
        }
        let (count, rows) = array(&owner.bytes, event + 0x40, 0x8080_6CC8, 24, 32)?;
        for index in 0..count {
            let at = rows + index * 24;
            if !at.is_multiple_of(8) {
                return Err(invalid("Akimbo particle selector is unaligned"));
            }
            if let Some(value) = selector(&owner, at, &bindings, endpoint, read)?
                && selectors.insert(at, value).is_some()
            {
                return Err(invalid("Akimbo firing events share a particle selector"));
            }
        }
    }
    if selectors.is_empty() || selectors.len() > 32 || !selectors.values().any(|s| s.role == 1) {
        return Err(invalid(
            "Akimbo needs 1–32 local firing selectors including a muzzle effect",
        ));
    }
    Ok((
        Fx {
            tag: owner.tag,
            bytes: owner.bytes.len() as u32,
            definition: owner.definition as u32,
            endpoint_binding: endpoint,
            selectors: selectors.into_values().collect(),
        },
        additional,
    ))
}

/// Assess the final graph through `read`, which must prefer pending allocations over stock.
/// `gear_entities` must contain every emitted appearance part, including marker-only parts.
pub(crate) fn assess_visual(
    entity: &[u8],
    group: u32,
    gear_entities: &[u32],
    pose: Option<&super::deployment::GunPose>,
    read: impl Fn(u32) -> AuthoringResult<Vec<u8>>,
) -> AuthoringResult<Visual> {
    if let Some(pose) = pose {
        self::pose::validate_marker(entity, pose, &read)?;
    } else {
        rig::assess(entity, &read)?;
    }
    let content = crate::weapon::behavior::content_with(entity, &read)?;
    let row = super::content::select(&content, group)?.block;
    let firing = firing_entity(
        read_u32(&content.owner, row + 0x60)?,
        read_u32(&content.owner, row + 0xC0)?,
        &read,
    )?;
    let (fx, additional) = effects(firing, &read)?;
    let (muzzle_bits, eject_bits, additional_markers, marker_bones) = attachment_frames(
        entity,
        gear_entities,
        pose,
        fx.selectors.iter().any(|selector| selector.role == 2),
        &additional,
        &read,
    )?;
    let (tracer_tag, tracer_definition) =
        tracer(entity, read_u32(&content.owner, row + 0xF0)?, &read)?;
    Ok(Visual {
        muzzle_bits,
        eject_bits,
        additional_markers,
        marker_bones,
        tracer_tag,
        tracer_definition,
        fx,
    })
}

#[cfg(test)]
mod tests;
