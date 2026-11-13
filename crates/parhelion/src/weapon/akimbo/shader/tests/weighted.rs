use super::*;

pub(super) fn fixture() -> Program {
    let mut signature = vec![0; 56];
    for (at, value) in [
        (0, 2u32),
        (4, 8),
        (8, 56),
        (20, 3),
        (24, 0),
        (32, 65),
        (44, 1),
        (48, 1),
    ] {
        signature[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    signature[28] = 15;
    signature[29] = 7;
    signature[52] = 15;
    signature[53] = 15;
    signature.extend_from_slice(b"POSITION\0BLENDINDICES\0");
    while !signature.len().is_multiple_of(4) {
        signature.push(0);
    }
    let mut cb_decl = ins(89 | (1 << 11), vec![cb(11, 24, 0xe4)]);
    cb_decl.args.clear();
    let declare = |register, mask| {
        let mut value = dst(register, mask);
        value.0[0] |= 1 << 12;
        ins(95, vec![value])
    };
    let mut output = dst(0, 15);
    output.0[0] |= 2 << 12;
    let mut temps = ins(104, vec![]);
    temps.words = vec![(2 << 24) | 104, 5];
    let mut instructions = vec![
        cb_decl,
        declare(0, 7),
        declare(1, 15),
        ins(101, vec![output.clone()]),
        temps,
        ins(
            38,
            vec![
                Operand(vec![13 << 12]),
                dst(0, 3),
                input(1, 0x04),
                vector([3, 3, 0, 0]),
            ],
        ),
        ins(
            35,
            vec![
                dst(1, 15),
                vector([3; 4]),
                input(1, 0x50),
                vector([1, 2, 1, 2]),
            ],
        ),
        ins(86, vec![dst(2, 3), input(1, 0xEE)]),
        ins(56, vec![dst(2, 3), temp(2, 0xe4), lit(1.0 / 255.0)]),
    ];
    let indices = [
        [
            Scalar {
                register: 0,
                lane: 0,
            },
            Scalar {
                register: 1,
                lane: 0,
            },
            Scalar {
                register: 1,
                lane: 1,
            },
        ],
        [
            Scalar {
                register: 0,
                lane: 1,
            },
            Scalar {
                register: 1,
                lane: 2,
            },
            Scalar {
                register: 1,
                lane: 3,
            },
        ],
    ];
    for axis in 0..3 {
        instructions.push(ins(
            56,
            vec![dst(3, 15), dynamic(indices[1][axis], 0xe4), temp(2, 0x55)],
        ));
        instructions.push(ins(
            50,
            vec![
                dst(3, 15),
                dynamic(indices[0][axis], 0xe4),
                temp(2, 0),
                temp(3, 0xe4),
            ],
        ));
        instructions.push(ins(
            17,
            vec![dst(4, 1 << axis), temp(3, 0xe4), input(0, 0x24)],
        ));
    }
    instructions.extend([ins(54, vec![output, temp(4, 0xe4)]), ins(62, vec![])]);
    Program::synthetic_vertex(instructions, signature, super::signature(0))
}

#[test]
fn two_bone_shader_requires_typed_inputs_complete_live_rows_and_unused_copy_lane() {
    let bytes = fixture().emit().unwrap();
    let output = transform_two_bone_vertex_with_source_palette(&bytes, contract(), 3).unwrap();
    assert!(transform_vertex_with_source_palette(&bytes, contract(), 3).is_err());
    assert!(transform_two_bone_vertex_with_source_palette(&output, contract(), 3).is_err());
    let mut changed = fixture();
    changed
        .instructions
        .insert(9, ins(54, vec![dst(4, 8), input(0, 0xff)]));
    assert!(
        transform_two_bone_vertex_with_source_palette(&changed.emit().unwrap(), contract(), 3)
            .is_err()
    );
    let mut changed = fixture();
    changed.instructions[5] = ins(
        38,
        vec![
            Operand(vec![13 << 12]),
            dst(0, 3),
            input(1, 0x04),
            vector([4, 3, 0, 0]),
        ],
    );
    assert!(
        transform_two_bone_vertex_with_source_palette(&changed.emit().unwrap(), contract(), 3)
            .is_err()
    );
    let mut changed = fixture();
    changed
        .instructions
        .insert(9, ins(54, vec![dst(0, 1), uint(0)]));
    assert!(
        transform_two_bone_vertex_with_source_palette(&changed.emit().unwrap(), contract(), 3)
            .is_err()
    );
    let mut changed = fixture();
    changed.instructions.drain(15..18);
    assert!(
        transform_two_bone_vertex_with_source_palette(&changed.emit().unwrap(), contract(), 3)
            .is_err()
    );
    let mut changed = fixture();
    changed.chunk_mut(b"ISGN").unwrap()[44..48].copy_from_slice(&3u32.to_le_bytes());
    assert!(
        transform_two_bone_vertex_with_source_palette(&changed.emit().unwrap(), contract(), 3)
            .is_err()
    );
}
