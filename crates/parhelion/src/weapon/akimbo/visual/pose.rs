//! A private marker channel built from the weapon's own equipment and arm rigs.
//!
//! Native FK range storage and all control/bone maps are extended together by the same
//! serializer used for imported rigs. The 72 shared arm bones are compared to the stock
//! adapter reference by name, parent and exact bind bytes before retaining its arm map.
use super::*;
use crate::weapon::akimbo::deployment::GunPose;
use parhelion_import::tiger::{animation::rig as native, payload::Payload};

pub(crate) const ARM_BONES: usize = 72;
const BASELINE: u32 = 0x80BC1000;
const IDENTITY: [f32; 8] = [0., 0., 0., 1., 0., 0., 0., 1.];
// FNV-1 name reserved for this private, unweighted protocol channel.
const MARKER_NAME: u32 = 0xEBD29F68; // FNV-1("parhelion_akimbo_marker")

pub(crate) struct Owner {
    pub tag: u32,
    pub original: Payload,
    pub converted: Vec<u8>,
    pub relocation: Option<(usize, usize)>,
}

pub(crate) struct Prepared {
    pub pose: GunPose,
    pub weapon_skeleton: Owner,
    pub weapon_controls: Owner,
    pub arms_skeleton: Owner,
    pub arms_controls: Owner,
    pub renumbering: native::Renumbering,
}

fn checked<T>(value: anyhow::Result<T>) -> AuthoringResult<T> {
    value.map_err(|error| invalid(format!("Akimbo native rig: {error:#}")))
}

fn range_contract(
    fk: &[usize],
    procedural: &[(usize, usize)],
    weapon_count: usize,
    arms: bool,
) -> AuthoringResult<usize> {
    let prefix = usize::from(arms);
    let root = if arms { ARM_BONES } else { 0 };
    let valid = fk.len() == prefix + 1
        && procedural.len() == prefix + 1
        && (!arms || (fk[0] == 25 && procedural[0] == (25, ARM_BONES - 25)));
    if !valid {
        return Err(invalid(
            "Akimbo rig has an unsupported FK/procedural range layout",
        ));
    }
    let count = fk[prefix];
    if count == 0
        || count >= weapon_count
        || procedural[prefix] != (root + count, weapon_count - count)
    {
        return Err(invalid(
            "Akimbo weapon FK/procedural ranges are not contiguous",
        ));
    }
    Ok(count)
}

fn range_layout(skeleton: &Payload, weapon_count: usize, arms: bool) -> AuthoringResult<usize> {
    let instance = checked(skeleton.pointer(16))?;
    let sections = checked(skeleton.array(instance + 0x30, 64, Some(0x80808544)))?;
    let procedural = checked(skeleton.array(instance + 0x40, 8, Some(0x80808A06)))?;
    let fk = sections
        .into_iter()
        .map(|at| checked(skeleton.array(at + 0x20, 32, Some(0x80809F75))).map(|poses| poses.len()))
        .collect::<AuthoringResult<Vec<_>>>()?;
    let procedural = procedural
        .into_iter()
        .map(|at| {
            Ok((
                usize::from(checked(skeleton.u16(at))?),
                usize::from(checked(skeleton.u16(at + 4))?),
            ))
        })
        .collect::<AuthoringResult<Vec<_>>>()?;
    range_contract(&fk, &procedural, weapon_count, arms)
}

fn rows(
    owner: &Component,
    field: usize,
    class: u32,
    stride: usize,
) -> AuthoringResult<(usize, usize)> {
    array(&owner.bytes, owner.definition + field, class, stride, 128)
}

fn arm_prefix(source: &Component, baseline: &Component) -> AuthoringResult<()> {
    let (count, nodes) = rows(source, 0x80, 0x80808A08, 16)?;
    let (reference_count, reference) = rows(baseline, 0x80, 0x80808A08, 16)?;
    let (frames, binds) = rows(source, 0x90, 0x80809F75, 32)?;
    let (_, reference_binds) = rows(baseline, 0x90, 0x80809F75, 32)?;
    let (inverse_count, inverse) = rows(source, 0xA0, 0x80809F75, 32)?;
    let (_, reference_inverse) = rows(baseline, 0xA0, 0x80809F75, 32)?;
    if count <= ARM_BONES
        || reference_count <= ARM_BONES
        || frames != count
        || inverse_count != count
    {
        return Err(invalid("Akimbo arm skeleton has inconsistent bone arrays"));
    }
    for bone in 0..ARM_BONES {
        // Child/sibling links may differ at the hand that owns the weapon (bows use the
        // left grip), while the shared arm hierarchy and calibrated binds stay identical.
        if source.bytes[nodes + bone * 16..nodes + bone * 16 + 8]
            != baseline.bytes[reference + bone * 16..reference + bone * 16 + 8]
            || source.bytes[binds + bone * 32..binds + (bone + 1) * 32]
                != baseline.bytes[reference_binds + bone * 32..reference_binds + (bone + 1) * 32]
            || source.bytes[inverse + bone * 32..inverse + (bone + 1) * 32]
                != baseline.bytes
                    [reference_inverse + bone * 32..reference_inverse + (bone + 1) * 32]
        {
            return Err(invalid(
                "Akimbo shared arm names, parents or bind transforms differ",
            ));
        }
    }
    Ok(())
}

fn owner(
    source: Component,
    converted: Payload,
    relocation: Option<(usize, usize)>,
) -> AuthoringResult<Owner> {
    let original = Payload(source.bytes);
    checked(native::validate_extension(
        &original, &converted, relocation,
    ))?;
    Ok(Owner {
        tag: source.tag,
        original,
        converted: converted.0,
        relocation,
    })
}

pub(super) fn validate_marker(
    entity: &[u8],
    pose: &GunPose,
    read: &Read<'_>,
) -> AuthoringResult<()> {
    let skeleton = rig::bound(entity, 0x1C80DD4A, 0x80808545, read)?;
    let controls = rig::bound(entity, 0x1DBD2776, 0x80808F96, read)?;
    let (count, nodes) = rows(&skeleton, 0x80, 0x80808A08, 16)?;
    let (frames, binds) = rows(&skeleton, 0x90, 0x80809F75, 32)?;
    let marker = pose.marker_bone as usize;
    if count != pose.weapon_bone_count as usize || count != frames || marker >= count {
        return Err(invalid(
            "Akimbo private marker palette differs from its descriptor",
        ));
    }
    let node = nodes + marker * 16;
    if read_u32(&skeleton.bytes, node)? != MARKER_NAME
        || read_u32(&skeleton.bytes, node + 4)? != 0
        || read_u32(&skeleton.bytes, node + 8)? != u32::MAX
        || !rig::same_frame(
            rig::frame_at(&skeleton.bytes, binds + marker * 32)?,
            pose.marker_bind_bits.map(f32::from_bits),
        )
    {
        return Err(invalid(
            "Akimbo private marker lost its independent identity bind",
        ));
    }
    checked(native::controls::validate(&controls.bytes))?;
    let (named, targets) = array(
        &controls.bytes,
        controls.definition + 0x110,
        0x80808FFE,
        8,
        256,
    )?;
    if (0..named)
        .any(|i| read_u32(&controls.bytes, targets + i * 8 + 4).ok() == Some(pose.marker_bone))
    {
        return Err(invalid(
            "Akimbo private marker is targeted by a native control",
        ));
    }
    Ok(())
}

pub(crate) fn prepare(
    entity: &[u8],
    arms: &[u8],
    read: impl Fn(u32) -> AuthoringResult<Vec<u8>>,
) -> AuthoringResult<Prepared> {
    let weapon = rig::bound(entity, 0x1C80DD4A, 0x80808545, &read)?;
    let controls = rig::bound(entity, 0x1DBD2776, 0x80808F96, &read)?;
    let combined = one_component(arms, 0x80808546, &read)?;
    let combined_controls = one_component(arms, 0x80808F8F, &read)?;
    let animation = one_component(arms, 0x808036CF, &read)?;
    let baseline = component(BASELINE, &read)?;
    if weapon.class != 0x80808546 || controls.class != 0x80808F8F || baseline.class != 0x80808546 {
        return Err(invalid("Akimbo weapon/arm rig resource classes differ"));
    }
    arm_prefix(&combined, &baseline)?;
    let equipment = Payload(weapon.bytes.clone());
    let full = Payload(combined.bytes.clone());
    let count = checked(native::bone_count(&equipment))?;
    if !(2..56).contains(&count)
        || checked(native::validate_weapon_pair(&equipment, &full))? != ARM_BONES
    {
        return Err(invalid(
            "Akimbo weapon rig exceeds the bounded shared-arm layout",
        ));
    }
    let fk = range_layout(&equipment, count, false)?;
    if range_layout(&full, count, true)? != fk {
        return Err(invalid("Akimbo equipment and arm FK ranges differ"));
    }
    let (_, weapon_frames) = rows(&weapon, 0x90, 0x80809F75, 32)?;
    if !rig::same_frame(rig::frame_at(&weapon.bytes, weapon_frames)?, IDENTITY) {
        return Err(invalid("Akimbo equipment root bind is not identity"));
    }
    let (_, full_frames) = rows(&combined, 0x90, 0x80809F75, 32)?;
    let full_root = rig::frame_at(&combined.bytes, full_frames + ARM_BONES * 32)?;
    let placement = checked(native::Placement::required(&equipment))?
        .max(checked(native::Placement::required(&full))?);
    let equipment_extra = native::Bone {
        name: MARKER_NAME,
        parent: 0,
        local: IDENTITY,
        object: IDENTITY,
        inverse: IDENTITY,
    };
    let full_extra = native::Bone {
        name: MARKER_NAME,
        parent: ARM_BONES,
        local: IDENTITY,
        object: full_root,
        inverse: rig::inverse(full_root),
    };
    let equipment_extension = checked(native::extend(
        &equipment,
        weapon.tag,
        &Payload(controls.bytes.clone()),
        &[equipment_extra],
        placement,
    ))?;
    let full_extension = checked(native::extend(
        &full,
        combined.tag,
        &Payload(combined_controls.bytes.clone()),
        &[full_extra],
        placement,
    ))?;
    if checked(native::validate_weapon_pair(
        &equipment_extension.skeleton,
        &full_extension.skeleton,
    ))? != ARM_BONES
        || equipment_extension
            .renumbering
            .map
            .iter()
            .enumerate()
            .any(|(old, &new)| full_extension.renumbering.map[ARM_BONES + old] != ARM_BONES + new)
        || full_extension.renumbering.map[..ARM_BONES]
            .iter()
            .copied()
            .ne(0..ARM_BONES)
        || equipment_extension.renumbering.first_appended != fk
        || range_layout(&equipment_extension.skeleton, count + 1, false)? != fk + 1
        || range_layout(&full_extension.skeleton, count + 1, true)? != fk + 1
    {
        return Err(invalid(
            "Akimbo equipment and arm marker extensions disagree",
        ));
    }
    let control_data = checked(full_extension.controls.pointer(24))?;
    let control_count = checked(full_extension.controls.array(
        control_data + 0xA0,
        52,
        Some(0x80809076),
    ))?
    .len();
    let frozen = animation.header + 0x50;
    let animation_bytes = read_u64(&animation.bytes, frozen + 8)?;
    if read_u32(&animation.bytes, frozen)? != animation.tag
        || read_u32(&animation.bytes, frozen + 4)? != 0x808036E9
        || !(0x100..=0x10000).contains(&animation_bytes)
    {
        return Err(invalid("Akimbo animation snapshot header differs"));
    }
    let pose = GunPose {
        weapon_bone_count: (count + 1) as u32,
        marker_bone: equipment_extension.renumbering.first_appended as u32,
        marker_bind_bits: IDENTITY.map(f32::to_bits),
        control_count: control_count as u32,
        animation_bytes: animation_bytes as u32,
    };
    Ok(Prepared {
        pose,
        weapon_skeleton: owner(
            weapon,
            equipment_extension.skeleton,
            Some(equipment_extension.relocation),
        )?,
        weapon_controls: owner(controls, equipment_extension.controls, None)?,
        arms_skeleton: owner(
            combined,
            full_extension.skeleton,
            Some(full_extension.relocation),
        )?,
        arms_controls: owner(combined_controls, full_extension.controls, None)?,
        renumbering: equipment_extension.renumbering,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::{append_native_array, write_u32};

    #[test]
    #[ignore = "requires clean SUNDIAL_STOCK_PACKAGES, SUNDIAL_AKIMBO_RIG_CASES census JSON and SUNDIAL_TEST_ARTIFACTS"]
    fn akimbo_stock_rig_extensions() {
        let packages = crate::test_support::stock_packages();
        sundial::package_authoring::initialize_source_decoder(&packages).unwrap();
        let manager =
            sundial::package_authoring::open_shadowkeep_package_manager(&packages).unwrap();
        let cases = std::fs::read(std::env::var_os("SUNDIAL_AKIMBO_RIG_CASES").unwrap()).unwrap();
        let cases: Vec<serde_json::Value> = serde_json::from_slice(&cases).unwrap();
        assert!(!cases.is_empty() && cases.len() <= 1024);
        let mut seen = BTreeSet::new();
        let mut report = Vec::new();
        for case in cases {
            let hash = |key: &str| u32::from_str_radix(case[key].as_str().unwrap(), 16).unwrap();
            if !seen.insert((hash("WeaponSkeleton"), hash("Skeleton"))) {
                continue;
            }
            let entity = manager.read_tag(hash("Entity")).unwrap();
            let arms = manager.read_tag(hash("Arms")).unwrap();
            let result = prepare(&entity, &arms, |tag| manager.read_tag(tag).map_err(invalid));
            let mut row = serde_json::json!({"name":case["Name"], "family":case["Family"],
                "weapon_skeleton":case["WeaponSkeleton"], "arms_skeleton":case["Skeleton"]});
            match result {
                Ok(prepared) => {
                    prepared.pose.validate().unwrap();
                    row["pose"] = serde_json::to_value(prepared.pose).unwrap();
                }
                Err(error) => row["error"] = error.to_string().into(),
            }
            report.push(row);
        }
        let output = crate::test_support::artifact_dir("akimbo-rig-extensions");
        assert!(
            !output.exists(),
            "Choose a fresh source-rig evidence directory"
        );
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(
            output.join("poses.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let failures = report
            .iter()
            .filter(|row| row.get("error").is_some())
            .collect::<Vec<_>>();
        assert!(
            failures.is_empty(),
            "Source rig extension failures: {failures:#?}"
        );
        println!(
            "Extended and validated {} distinct source rig pairs",
            report.len()
        );
    }

    #[test]
    fn akimbo_fk_ranges_preserve_native_children_before_the_marker() {
        for (fk, count) in [(1, 5), (2, 4), (3, 25), (5, 6), (12, 15)] {
            assert_eq!(
                range_contract(&[fk], &[(fk, count - fk)], count, false).unwrap(),
                fk
            );
            assert_eq!(
                range_contract(&[25, fk], &[(25, 47), (72 + fk, count - fk)], count, true).unwrap(),
                fk
            );
            assert_eq!(
                range_contract(
                    &[25, fk + 1],
                    &[(25, 47), (73 + fk, count - fk)],
                    count + 1,
                    true
                )
                .unwrap(),
                fk + 1
            );
        }
        assert!(range_contract(&[25, 2], &[(25, 47), (73, 2)], 4, true).is_err());
        assert!(range_contract(&[24, 2], &[(24, 48), (74, 2)], 4, true).is_err());
        assert!(range_contract(&[4], &[(4, 0)], 4, false).is_err());
        assert!(range_contract(&[2], &[(3, 2)], 4, false).is_err());
    }

    fn arms(count: usize) -> Component {
        let mut result = Component {
            tag: 1,
            bytes: vec![0; 0x200],
            header: 0x80,
            definition: 0x100,
            class: 0x80808546,
        };
        let mut nodes = vec![0; count * 16];
        for bone in 0..count {
            write_u32(&mut nodes, bone * 16, 1000 + bone as u32).unwrap();
            write_u32(
                &mut nodes,
                bone * 16 + 4,
                if bone == 0 { u32::MAX } else { 0 },
            )
            .unwrap();
        }
        append_native_array(&mut result.bytes, 0x180, 0x80808A08, count, &nodes).unwrap();
        let frames = (0..count)
            .flat_map(|_| IDENTITY.into_iter().flat_map(f32::to_le_bytes))
            .collect::<Vec<_>>();
        for field in [0x190, 0x1A0] {
            append_native_array(&mut result.bytes, field, 0x80809F75, count, &frames).unwrap();
        }
        result
    }

    #[test]
    fn akimbo_shared_arms_require_names_parents_and_both_bind_arrays() {
        let baseline = arms(77);
        let mut candidate = arms(97);
        arm_prefix(&candidate, &baseline).unwrap();
        let (_, nodes) = rows(&candidate, 0x80, 0x80808A08, 16).unwrap();
        // The weapon may be parented to the other hand without changing its arm contract.
        write_u32(&mut candidate.bytes, nodes + 23 * 16 + 8, 72).unwrap();
        arm_prefix(&candidate, &baseline).unwrap();
        for (field, stride, word) in [(0x80, 16, 0), (0x80, 16, 4), (0x90, 32, 16), (0xA0, 32, 16)]
        {
            let mut changed = arms(97);
            let (_, at) = rows(
                &changed,
                field,
                if stride == 16 { 0x80808A08 } else { 0x80809F75 },
                stride,
            )
            .unwrap();
            write_u32(&mut changed.bytes, at + 23 * stride + word, 0xDEADBEEF).unwrap();
            assert!(arm_prefix(&changed, &baseline).is_err());
        }
    }
}
