use super::*;
use crate::tag_payload::append_native_array;

#[test]
fn animated_attachment_preserves_bone_local_frame_and_source_bind() {
    let identity = [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()];
    let mut bind = identity;
    bind[4] = 0.4f32.to_bits();
    let mut local = identity;
    local[5] = (-0.1f32).to_bits();
    local[7] = 1.4256907f32.to_bits();
    let mut rows = vec![Marker {
        availability: 7,
        bone: 2,
        bits: local,
    }];
    let mut bindings = Vec::new();
    assert_eq!(
        marker_frame(
            Some(&rows),
            "muzzle",
            1,
            0,
            &[identity, identity, bind],
            &mut bindings
        )
        .unwrap(),
        local
    );
    assert_eq!(
        bindings,
        vec![MarkerBone {
            role: 1,
            index: 0,
            bone: 2,
            bind_bits: bind
        }]
    );
    rows[0].bone = 0;
    bindings.clear();
    assert_eq!(
        marker_frame(Some(&rows), "muzzle", 1, 0, &[identity], &mut bindings).unwrap(),
        local
    );
    assert!(bindings.is_empty());
    rows[0].bone = 3;
    assert!(marker_frame(Some(&rows), "muzzle", 1, 0, &[identity; 3], &mut bindings).is_err());
    rows[0].bone = 2;
    rows.push(Marker {
        availability: 8,
        bone: 2,
        bits: local,
    });
    assert!(marker_frame(Some(&rows), "muzzle", 1, 0, &[identity; 3], &mut bindings).is_err());
    assert!(bindings.is_empty());
}

#[test]
fn missing_ejection_is_allowed_only_without_ejection_selectors() {
    assert!(ejection_frame(None, true).is_err());
    let unused = ejection_frame(None, false).unwrap();
    assert_eq!(
        unused,
        [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()]
    );
    let marker = Marker {
        availability: 0,
        bone: 0,
        bits: unused,
    };
    assert_eq!(
        ejection_frame(Some(&vec![marker.clone()]), true).unwrap(),
        unused
    );
    assert_eq!(
        ejection_frame(Some(&vec![marker.clone(), marker]), false).unwrap(),
        unused
    );
}

#[test]
fn resolves_the_native_firing_slot_without_using_idle_or_reload_entities() {
    let mut wrapper = vec![0; 24];
    put32(&mut wrapper, 16, 8);
    finish(&mut wrapper);
    let mut overrides = vec![0; 112];
    put64(&mut overrides, 8, 0x811C_9DC5);
    for (index, at) in (24..112).step_by(16).enumerate() {
        put32(&mut overrides, at, 0x80BB0001 + index as u32);
    }
    finish(&mut overrides);
    let read = |tag| match tag {
        7 => Ok(wrapper.clone()),
        8 => Ok(overrides.clone()),
        _ => Err(invalid("Unexpected graph read")),
    };
    assert_eq!(
        firing_entity(0x80BB0010, u32::MAX, &read).unwrap(),
        0x80BB0010
    );
    assert_eq!(firing_entity(u32::MAX, 7, &read).unwrap(), 0x80BB0001);
    assert!(firing_entity(0x80BB0010, 7, &read).is_err());
    for mutation in 0..6 {
        let mut root = wrapper.clone();
        let mut list = overrides.clone();
        match mutation {
            0 => put64(&mut root, 0, 23),
            1 => put64(&mut root, 8, 1),
            2 => put64(&mut list, 8, 0),
            3 => put32(&mut list, 24, u32::MAX),
            4 => put64(&mut list, 16, u64::MAX / 2),
            _ => put32(&mut list, 108, 1),
        }
        let read = |tag| match tag {
            7 => Ok(root.clone()),
            8 => Ok(list.clone()),
            _ => Err(invalid("Missing fixture")),
        };
        assert!(
            firing_entity(u32::MAX, 7, &read).is_err(),
            "mutation {mutation}"
        );
    }
}

fn projectile_fixture() -> BTreeMap<u32, Vec<u8>> {
    let mut entity = vec![0; 0x40];
    let mut row = [0; 12];
    put32(&mut row, 0, 12);
    append_native_array(&mut entity, 0x10, 0x80809C04, 1, &row).unwrap();
    let mut movement = vec![0; 0x200];
    pointer(&mut movement, 0x10, 0x130);
    pointer(&mut movement, 0x18, 0x180);
    put32(&mut movement, 0x130, 12);
    put32(&mut movement, 0x134, 0x8080388F);
    put64(&mut movement, 0x138, 0x180);
    finish(&mut movement);
    BTreeMap::from([(11, entity), (12, movement)])
}

#[test]
fn selected_projectile_override_resolves_its_own_typed_movement_owner() {
    let assets = projectile_fixture();
    let read = |tag| {
        assets
            .get(&tag)
            .cloned()
            .ok_or_else(|| invalid("No fixture asset"))
    };
    // No barrel exists in this entity: a valid selected override is authoritative.
    assert_eq!(tracer(&[], 11, &read).unwrap(), (12, 0x180));
    assert!(tracer(&[], u32::MAX, &read).is_err());
    assert!(tracer(&[], 0, &read).is_err());
    assert!(tracer(&[], 13, &read).is_err());
}

#[test]
fn projectile_override_keeps_layout_ownership_and_uniqueness_guards() {
    for mutation in 0..4 {
        let mut assets = projectile_fixture();
        match mutation {
            0 => put32(assets.get_mut(&12).unwrap(), 0x134, 0x80801234),
            1 => put32(assets.get_mut(&12).unwrap(), 0x130, 13),
            2 => {
                pointer(assets.get_mut(&12).unwrap(), 0x18, 0x181);
                put64(assets.get_mut(&12).unwrap(), 0x138, 0x181);
            }
            _ => {
                let mut rows = [0; 24];
                put32(&mut rows, 0, 12);
                put32(&mut rows, 12, 12);
                append_native_array(assets.get_mut(&11).unwrap(), 0x10, 0x80809C04, 2, &rows)
                    .unwrap();
            }
        }
        let read = |tag| {
            assets
                .get(&tag)
                .cloned()
                .ok_or_else(|| invalid("No fixture asset"))
        };
        assert!(tracer(&[], 11, &read).is_err(), "mutation {mutation}");
    }
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn pointer(bytes: &mut [u8], at: usize, target: usize) {
    bytes[at..at + 8].copy_from_slice(&(target as i64 - at as i64).to_le_bytes());
}

fn finish(bytes: &mut [u8]) {
    put64(bytes, 0, bytes.len() as u64);
}

struct Fixture {
    assets: BTreeMap<u32, Vec<u8>>,
    selector: usize,
    binding: usize,
    event: usize,
}

impl Fixture {
    fn new() -> Self {
        let mut sequence = vec![0; 0x500];
        pointer(&mut sequence, 0x10, 0x90);
        pointer(&mut sequence, 0x18, 0x200);
        put32(&mut sequence, 0x90, 2);
        put32(&mut sequence, 0x94, 0x808084e9);
        put64(&mut sequence, 0x98, 0x200);
        let mut bindings = vec![0; 32];
        bindings[0] = 3;
        bindings[2] = 1;
        put32(&mut bindings, 4, 0x22);
        put32(&mut bindings, 8, MUZZLE);
        put32(&mut bindings, 12, 0xFF);
        bindings[16] = 1;
        bindings[17] = 1;
        bindings[18] = 1;
        put32(&mut bindings, 20, 0x5F);
        put32(&mut bindings, 28, 0xFF);
        append_native_array(&mut sequence, 0x3C0, 0x80808615, 2, &bindings).unwrap();
        let (_, _, binding, _) = array_at(&sequence, 0x3C0).unwrap();
        append_native_array(&mut sequence, 0x368, 0x808093E6, 1, &[0; 24]).unwrap();
        let (_, _, events, _) = array_at(&sequence, 0x368).unwrap();
        let event = 0x400;
        pointer(&mut sequence, events + 16, event);
        put32(&mut sequence, event - 4, 0x80806CC6);
        append_native_array(&mut sequence, event + 0x40, 0x80806CC8, 1, &[0; 24]).unwrap();
        let (_, _, selector, _) = array_at(&sequence, event + 0x40).unwrap();
        append_native_array(&mut sequence, selector, 0x80800009, 2, &[0, 1]).unwrap();
        put32(&mut sequence, selector + 16, 3);
        finish(&mut sequence);
        let mut entity = vec![0; 0x40];
        let mut row = [0; 12];
        put32(&mut row, 0, 2);
        append_native_array(&mut entity, 0x10, 0x80809C04, 1, &row).unwrap();
        let mut particle = vec![0; 16];
        put32(&mut particle, 0, 4);
        let mut program = vec![0; 0x80];
        put64(&mut program, 0x18, 2);
        program[0x38..0x40].copy_from_slice(&[0, 1, 1, 1, 0, 0, 0, 0]);
        program[0x47] = 2;
        finish(&mut program);
        Self {
            assets: BTreeMap::from([(1, entity), (2, sequence), (3, particle), (4, program)]),
            selector,
            binding,
            event,
        }
    }

    fn read(&self, tag: u32) -> AuthoringResult<Vec<u8>> {
        self.assets
            .get(&tag)
            .cloned()
            .ok_or_else(|| invalid("Missing synthetic visual asset"))
    }

    fn assess(&self) -> AuthoringResult<Fx> {
        effects(1, &|tag| self.read(tag)).map(|value| value.0)
    }

    fn owner(&mut self) -> &mut Vec<u8> {
        self.assets.get_mut(&2).unwrap()
    }
}

#[test]
fn derives_selector_offsets_and_program_from_typed_graph() {
    let fixture = Fixture::new();
    let fx = fixture.assess().unwrap();
    assert_eq!(fx.tag, 2);
    assert_eq!(fx.definition, 0x200);
    assert_eq!(fx.endpoint_binding, 1);
    assert_eq!(
        fx.selectors,
        vec![FxSelector {
            offset: fixture.selector as u32,
            particle: 3,
            binding: 0,
            inputs: 2,
            markers: 1,
            groups: 2,
            program: 4,
            program_bytes: 0x80,
            role: 1,
        }]
    );
}

#[test]
fn akimbo_multi_marker_attachment_keeps_native_capacity_and_ordered_role() {
    let mut fixture = Fixture::new();
    let mut bindings = fixture.assets[&2][fixture.binding..fixture.binding + 32].to_vec();
    bindings.resize(48, 0);
    bindings[32] = 3;
    bindings[34] = 6;
    put32(&mut bindings, 36, 0x22);
    put32(&mut bindings, 40, 0x76BCE859);
    put32(&mut bindings, 44, 0xFF);
    append_native_array(fixture.owner(), 0x3C0, 0x80808615, 3, &bindings).unwrap();
    let event = fixture.event;
    append_native_array(fixture.owner(), event + 0x40, 0x80806CC8, 2, &[0; 48]).unwrap();
    let (_, _, selectors, _) = array_at(&fixture.assets[&2], event + 0x40).unwrap();
    for (index, inputs) in [vec![0u8, 1], vec![2]].into_iter().enumerate() {
        let at = selectors + index * 24;
        append_native_array(fixture.owner(), at, 0x80800009, inputs.len(), &inputs).unwrap();
        put32(fixture.owner(), at + 16, 3);
    }
    finish(fixture.owner());
    let (effects, names) = effects(1, &|tag| fixture.read(tag)).unwrap();
    assert_eq!(names, [0x76BCE859]);
    assert_eq!(effects.selectors[0].role, 1);
    assert_eq!(effects.selectors[1].role, 3);
    assert_eq!(effects.selectors[1].markers, 6);
    assert_eq!(effects.selectors[1].inputs, 1);
}

#[test]
fn rejects_unsupported_particle_mapping_and_capacity() {
    let mut fixture = Fixture::new();
    fixture.assets.get_mut(&4).unwrap()[0x39] = 2;
    assert!(fixture.assess().is_err());
    let mut fixture = Fixture::new();
    let at = fixture.binding + 2;
    fixture.owner()[at] = 3;
    assert!(fixture.assess().is_err());
}

#[test]
fn rejects_wrong_endpoint_and_additional_local_marker() {
    let mut fixture = Fixture::new();
    let (_, _, rows, _) = array_at(&fixture.assets[&2], fixture.selector).unwrap();
    fixture.owner()[rows + 1] = 0;
    assert!(fixture.assess().is_err());
    let mut fixture = Fixture::new();
    let at = fixture.binding + 8;
    put32(fixture.owner(), at, 0x1234);
    assert!(fixture.assess().is_err());
}

#[test]
fn rejects_unknown_events_and_malformed_relative_arrays() {
    let mut fixture = Fixture::new();
    let at = fixture.event - 4;
    put32(fixture.owner(), at, 0x80801234);
    assert!(fixture.assess().is_err());
    let mut fixture = Fixture::new();
    put64(fixture.owner(), 0x3C8, u64::MAX / 2);
    assert!(fixture.assess().is_err());
}

#[test]
fn rejects_ambiguous_component_and_inconsistent_self_reference() {
    let mut fixture = Fixture::new();
    let mut rows = [0; 24];
    put32(&mut rows, 0, 2);
    put32(&mut rows, 12, 2);
    append_native_array(
        fixture.assets.get_mut(&1).unwrap(),
        0x10,
        0x80809C04,
        2,
        &rows,
    )
    .unwrap();
    assert!(fixture.assess().is_err());
    let mut fixture = Fixture::new();
    put32(fixture.owner(), 0x90, 5);
    assert!(fixture.assess().is_err());
}

#[test]
fn rejects_nonroot_duplicate_and_invalid_attachment_frames() {
    let valid = Marker {
        availability: 7,
        bone: 0,
        bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
    };
    assert_eq!(
        frame(Some(&vec![valid.clone()]), "muzzle").unwrap(),
        valid.bits
    );
    let mut alternate = valid.clone();
    alternate.bits[4] = 0.5f32.to_bits();
    assert_eq!(
        frame(Some(&vec![valid.clone(), alternate.clone()]), "muzzle").unwrap(),
        valid.bits
    );
    assert_eq!(
        frame(Some(&vec![alternate.clone(), valid.clone()]), "muzzle").unwrap(),
        alternate.bits
    );
    alternate.availability += 1;
    assert!(frame(Some(&vec![valid.clone(), alternate]), "muzzle").is_err());
    let mut bad = valid.clone();
    bad.bone = 1;
    assert!(frame(Some(&vec![bad]), "muzzle").is_err());
    let mut bad = valid;
    bad.bits[4] = f32::NAN.to_bits();
    assert!(frame(Some(&vec![bad]), "muzzle").is_err());
}

#[test]
fn rejects_pose_bone_used_by_any_attachment_name() {
    let mut owner = Component {
        tag: 1,
        bytes: vec![0; 0x200],
        header: 0x80,
        definition: 0x100,
        class: 0x80808507,
    };
    put32(&mut owner.bytes, 0x7C, 0x80808506);
    let mut row = vec![0; 64];
    put32(&mut row, 0, 17);
    put32(&mut row, 4, 3);
    put32(&mut row, 0x30, 0x12345678);
    append_native_array(&mut owner.bytes, 0x1B0, 0x80808513, 1, &row).unwrap();
    assert!(markers(&owner, 3, &[]).is_err());
    put32(&mut row, 0, 3);
    put32(&mut row, 4, 0);
    put32(&mut row, 0x30, MUZZLE);
    append_native_array(&mut owner.bytes, 0x1B0, 0x80808513, 1, &row).unwrap();
    let parsed = markers(&owner, 3, &[]).unwrap();
    assert_eq!(parsed[&MUZZLE][0].availability, 3);
    assert_eq!(parsed[&MUZZLE][0].bone, 0);
    let mut unsorted = row.clone();
    put32(&mut row, 0x30, 1);
    unsorted.extend(row);
    append_native_array(&mut owner.bytes, 0x1B0, 0x80808513, 2, &unsorted).unwrap();
    assert!(
        markers(&owner, 3, &[])
            .unwrap_err()
            .to_string()
            .contains("lookup order")
    );
}
