//! Prove that the native five-bone weapon rig can lend bone 3 to the pose protocol.
use super::*;
use crate::tag_payload::read_u16;

const BONE_NAMES: [u32; 5] = [0xC410084A, 0xD7A79DD5, 0x8CE8ED52, 0xB36093C2, 0x6011E092];
// This is the native adapter's smg::weaponBind3 contract, not a donor model template.
const MARKER_BIND: [f32; 8] = [
    0.50000006,
    0.5,
    0.50000006,
    0.50000006,
    0.053819977,
    0.0,
    0.15486507,
    1.0,
];
type Frame = [f32; 8];

fn exactly(
    owner: &Component,
    field: usize,
    class: u32,
    stride: usize,
    count: usize,
) -> AuthoringResult<usize> {
    let (actual, rows) = array(&owner.bytes, owner.definition + field, class, stride, count)?;
    if actual != count {
        return Err(invalid(
            "Akimbo weapon rig has a different bone/control count",
        ));
    }
    Ok(rows)
}

pub(super) fn frame_at(bytes: &[u8], at: usize) -> AuthoringResult<Frame> {
    let mut value = [0.; 8];
    for (index, field) in value.iter_mut().enumerate() {
        *field = f32::from_bits(read_u32(bytes, at + index * 4)?);
    }
    let norm: f32 = value[..4].iter().map(|v| v * v).sum();
    if value.iter().any(|v| !v.is_finite())
        || (norm - 1.).abs() > 0.00002
        || (value[7] - 1.).abs() > 0.000002
    {
        return Err(invalid(
            "Akimbo weapon rig has an unsupported bind transform",
        ));
    }
    Ok(value)
}

pub(super) fn same_frame(a: Frame, b: Frame) -> bool {
    let sign = if a[..4].iter().zip(&b[..4]).map(|(a, b)| a * b).sum::<f32>() < 0. {
        -1.
    } else {
        1.
    };
    (0..8).all(|i| (a[i] - b[i] * if i < 4 { sign } else { 1. }).abs() < 0.000002)
}

pub(super) fn inverse(frame: Frame) -> Frame {
    let mut q = [-frame[0], -frame[1], -frame[2], frame[3]];
    let norm: f32 = q.iter().map(|v| v * v).sum();
    for v in &mut q {
        *v /= norm;
    }
    let v = [-frame[4], -frame[5], -frame[6]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let xyz = [q[0], q[1], q[2]];
    let t = cross(xyz, v).map(|x| 2. * x);
    let c = cross(xyz, t);
    [
        q[0],
        q[1],
        q[2],
        q[3],
        v[0] + q[3] * t[0] + c[0],
        v[1] + q[3] * t[1] + c[1],
        v[2] + q[3] * t[2] + c[2],
        1.,
    ]
}

fn skeleton(owner: &Component) -> AuthoringResult<[Frame; 5]> {
    if owner.class != 0x80808546 {
        return Err(invalid("Akimbo weapon skeleton class differs"));
    }
    let nodes = exactly(owner, 0x80, 0x80808A08, 16, 5)?;
    let object = exactly(owner, 0x90, 0x80809F75, 32, 5)?;
    let inverted = exactly(owner, 0xA0, 0x80809F75, 32, 5)?;
    let mut frames = [[0.; 8]; 5];
    for (index, name) in BONE_NAMES.iter().enumerate() {
        let row = nodes + index * 16;
        let parent = if index == 0 { u32::MAX } else { 0 };
        if read_u32(&owner.bytes, row)? != *name
            || read_u32(&owner.bytes, row + 4)? != parent
            || (index != 0 && read_u32(&owner.bytes, row + 8)? != u32::MAX)
        {
            return Err(invalid(
                "Akimbo weapon skeleton has different bone roles or a dependent marker bone",
            ));
        }
        frames[index] = frame_at(&owner.bytes, object + index * 32)?;
        if !same_frame(
            inverse(frames[index]),
            frame_at(&owner.bytes, inverted + index * 32)?,
        ) {
            return Err(invalid(
                "Akimbo weapon inverse-bind rows disagree with its skeleton",
            ));
        }
    }
    if !same_frame(frames[0], [0., 0., 0., 1., 0., 0., 0., 1.])
        || !same_frame(frames[3], MARKER_BIND)
    {
        return Err(invalid(
            "Akimbo weapon marker bind differs from the native pose adapter",
        ));
    }
    Ok(frames)
}

fn controls(owner: &Component, frames: [Frame; 5]) -> AuthoringResult<()> {
    if owner.class != 0x80808F8F {
        return Err(invalid("Akimbo weapon control class differs"));
    }
    let rows = exactly(owner, 0xA0, 0x80809076, 52, 5)?;
    let defaults = exactly(owner, 0xB0, 0x80809F75, 32, 5)?;
    let forward = exactly(owner, 0xD8, 0x80800006, 2, 5)?;
    let reverse = exactly(owner, 0xE8, 0x80800006, 2, 5)?;
    for (index, frame) in frames.iter().enumerate() {
        let row = rows + index * 52;
        let operation = if index == 0 { 0xADB0A86B } else { 0x7BFA6624 };
        let parent = if index == 0 { u16::MAX } else { 0 };
        if read_u32(&owner.bytes, row)? != operation
            || read_u16(&owner.bytes, row + 44)? != index as u16
            || read_u16(&owner.bytes, forward + index * 2)? != index as u16
            || read_u16(&owner.bytes, reverse + index * 2)? != index as u16
            || [4, 10, 24, 30]
                .into_iter()
                .any(|offset| read_u16(&owner.bytes, row + offset).ok() != Some(parent))
            || !same_frame(*frame, frame_at(&owner.bytes, defaults + index * 32)?)
        {
            return Err(invalid(
                "Akimbo weapon controls change bone ordering, parent roles or bind defaults",
            ));
        }
    }
    let (count, names) = array(&owner.bytes, owner.definition + 0x110, 0x80808FFE, 8, 256)?;
    for index in 0..count {
        let bone = read_u32(&owner.bytes, names + index * 8 + 4)?;
        if bone >= 5 || bone == 3 {
            return Err(invalid(
                "Akimbo pose marker is targeted by a named native control",
            ));
        }
    }
    Ok(())
}

pub(super) fn bound(
    entity: &[u8],
    key: u32,
    class: u32,
    read: &Read<'_>,
) -> AuthoringResult<Component> {
    let bindings = weapon_component_bindings(entity, key).map_err(invalid)?;
    let [binding] = bindings.as_slice() else {
        return Err(invalid("Akimbo weapon rig binding is ambiguous"));
    };
    let owner = component(binding.owner_tag, read)?;
    if binding.resource_count != 1
        || binding.concrete_class != class
        || binding.resource_offset != owner.header as u64
    {
        return Err(invalid(
            "Akimbo weapon rig binding and native component disagree",
        ));
    }
    Ok(owner)
}

pub(super) fn assess(entity: &[u8], read: &Read<'_>) -> AuthoringResult<()> {
    let frames = skeleton(&bound(entity, 0x1C80DD4A, 0x80808545, read)?)?;
    controls(&bound(entity, 0x1DBD2776, 0x80808F96, read)?, frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::append_native_array;

    fn put(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn owner(class: u32) -> Component {
        Component {
            tag: 1,
            bytes: vec![0; 0x200],
            header: 0x80,
            definition: 0x20,
            class,
        }
    }
    fn packed(frames: &[Frame]) -> Vec<u8> {
        frames
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
    fn fixture() -> (Component, Component) {
        let mut frames = [[0., 0., 0., 1., 0., 0., 0., 1.]; 5];
        frames[3] = MARKER_BIND;
        let mut skeleton = owner(0x80808546);
        let mut hierarchy = vec![0; 5 * 16];
        for (index, name) in BONE_NAMES.into_iter().enumerate() {
            put(&mut hierarchy, index * 16, name);
            put(
                &mut hierarchy,
                index * 16 + 4,
                if index == 0 { u32::MAX } else { 0 },
            );
            put(
                &mut hierarchy,
                index * 16 + 8,
                if index == 0 { 4 } else { u32::MAX },
            );
        }
        append_native_array(&mut skeleton.bytes, 0xA0, 0x80808A08, 5, &hierarchy).unwrap();
        append_native_array(&mut skeleton.bytes, 0xB0, 0x80809F75, 5, &packed(&frames)).unwrap();
        append_native_array(
            &mut skeleton.bytes,
            0xC0,
            0x80809F75,
            5,
            &packed(&frames.map(inverse)),
        )
        .unwrap();
        let mut controls = owner(0x80808F8F);
        let mut operations = vec![0; 5 * 52];
        for index in 0..5 {
            put(
                &mut operations,
                index * 52,
                if index == 0 { 0xADB0A86B } else { 0x7BFA6624 },
            );
            for field in [4, 10, 24, 30] {
                operations[index * 52 + field..index * 52 + field + 2]
                    .copy_from_slice(&(if index == 0 { u16::MAX } else { 0 }).to_le_bytes());
            }
            operations[index * 52 + 44..index * 52 + 46]
                .copy_from_slice(&(index as u16).to_le_bytes());
        }
        append_native_array(&mut controls.bytes, 0xC0, 0x80809076, 5, &operations).unwrap();
        append_native_array(&mut controls.bytes, 0xD0, 0x80809F75, 5, &packed(&frames)).unwrap();
        let indices = (0u16..5).flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        for field in [0xF8, 0x108] {
            append_native_array(&mut controls.bytes, field, 0x80800006, 5, &indices).unwrap();
        }
        append_native_array(&mut controls.bytes, 0x130, 0x80808FFE, 1, &[0; 8]).unwrap();
        (skeleton, controls)
    }

    #[test]
    fn validates_marker_bind_and_rejects_dependent_bones() {
        let (mut source, control) = fixture();
        let frames = skeleton(&source).unwrap();
        controls(&control, frames).unwrap();
        let nodes = exactly(&source, 0x80, 0x80808A08, 16, 5).unwrap();
        put(&mut source.bytes, nodes + 4 * 16 + 4, 3);
        assert!(skeleton(&source).is_err());
        let (mut source, _) = fixture();
        let rows = exactly(&source, 0x90, 0x80809F75, 32, 5).unwrap();
        put(&mut source.bytes, rows + 3 * 32 + 16, 1.0f32.to_bits());
        assert!(skeleton(&source).is_err());
    }

    #[test]
    fn rejects_remapped_controls_and_named_pose_marker() {
        let (source, mut control) = fixture();
        let frames = skeleton(&source).unwrap();
        let rows = exactly(&control, 0xE8, 0x80800006, 2, 5).unwrap();
        control.bytes[rows + 6] = 2;
        assert!(controls(&control, frames).is_err());
        let (source, mut control) = fixture();
        let frames = skeleton(&source).unwrap();
        let (_, rows) = array(&control.bytes, 0x130, 0x80808FFE, 8, 256).unwrap();
        put(&mut control.bytes, rows + 4, 3);
        assert!(controls(&control, frames).is_err());
    }
}
