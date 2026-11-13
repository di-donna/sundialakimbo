use super::*;

#[path = "tests/execution.rs"]
mod execution;
#[path = "tests/weighted.rs"]
mod weighted;

fn input(register: u32, swizzle: u32) -> Operand {
    let mut value = temp(register, swizzle);
    value.0[0] |= 1 << 12;
    value
}

fn uint(value: u32) -> Operand {
    Operand(vec![1 | (4 << 12), value])
}
fn vector(values: [u32; 4]) -> Operand {
    let mut words = vec![2 | (4 << 12)];
    words.extend(values);
    Operand(words)
}

fn dynamic(index: Scalar, swizzle: u32) -> Operand {
    let mut words = vec![
        2 | (1 << 2) | (swizzle << 4) | (8 << 12) | (2 << 20) | (3 << 25),
        11,
        8,
    ];
    words.extend(index.source().0);
    Operand(words)
}

fn signature(register: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 32];
    for (at, value) in [(0, 1u32), (4, 8), (8, 32), (20, 3), (24, register)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[28] = 15;
    bytes[29] = 15;
    bytes.extend_from_slice(b"POSITION\0");
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn stock(register: u32, input_register: u32) -> Program {
    let bone = Scalar { register, lane: 0 };
    let mut cb_decl = ins(89 | (1 << 11), vec![cb(11, 24, 0xe4)]);
    cb_decl.args.clear();
    let mut temps = ins(104, vec![]);
    temps.words = vec![(2 << 24) | 104, register + 4];
    let mut input_decl = dst(input_register, 15);
    input_decl.0[0] |= 1 << 12;
    let mut output = dst(0, 15);
    output.0[0] |= 2 << 12;
    let instructions = vec![
        cb_decl,
        ins(95, vec![input_decl]),
        ins(101, vec![output.clone()]),
        temps,
        ins(
            50,
            vec![
                bone.destination(),
                input(input_register, 0xff),
                lit(32767.0),
                lit(0.1),
            ],
        ),
        ins(28, vec![bone.destination(), bone.source()]),
        ins(
            38,
            vec![
                Operand(vec![13 << 12]),
                dst(register, 2),
                bone.source(),
                uint(3),
            ],
        ),
        ins(
            35,
            vec![
                dst(register, 5),
                vector([3, 0, 3, 0]),
                bone.source(),
                vector([1, 0, 2, 0]),
            ],
        ),
        ins(
            17,
            vec![
                dst(register + 1, 1),
                dynamic(Scalar { register, lane: 1 }, 0xe4),
                input(input_register, 0x24),
            ],
        ),
        ins(
            17,
            vec![
                dst(register + 1, 2),
                dynamic(Scalar { register, lane: 0 }, 0xe4),
                input(input_register, 0x24),
            ],
        ),
        ins(
            17,
            vec![
                dst(register + 1, 4),
                dynamic(Scalar { register, lane: 2 }, 0xe4),
                input(input_register, 0x24),
            ],
        ),
        ins(54, vec![output, temp(register + 1, 0xe4)]),
        ins(62, vec![]),
    ];
    Program::synthetic_vertex(instructions, signature(input_register), signature(0))
}

fn contract() -> VertexContract {
    VertexContract {
        marker_bone: 3,
        bone_palette: 4,
        separation: 0.36,
    }
}

#[test]
fn shader_generation_uses_structure_not_donor_or_register_ids() {
    for (register, input) in [(0, 0), (7, 2), (31, 7)] {
        let source = stock(register, input).emit().unwrap();
        let output = transform_vertex(&source, contract()).unwrap();
        assert_eq!(transform_vertex(&source, contract()).unwrap(), output);
        let before = Program::read_vertex(&source).unwrap();
        let after = Program::read_vertex(&output).unwrap();
        assert_eq!(before.chunk(b"ISGN"), after.chunk(b"ISGN"));
        assert_eq!(before.chunk(b"OSGN"), after.chunk(b"OSGN"));
        assert_eq!(after.instructions[3].words[1], register + 23);
        assert_eq!(after.instructions[0].words[3], 32);
        assert!(Program::read(&source).is_err());
        assert!(Program::read(&output).is_err());
        assert!(
            transform_vertex(&output, contract()).is_err(),
            "Do not patch a shader twice"
        );
    }
}

#[test]
fn changed_marker_and_separation_change_only_generated_behavior() {
    let source = stock(0, 0).emit().unwrap();
    let a = transform_vertex(&source, contract()).unwrap();
    let b = transform_vertex(
        &source,
        VertexContract {
            marker_bone: 1,
            separation: 0.19,
            ..contract()
        },
    )
    .unwrap();
    assert_ne!(a, b);
    assert_eq!(
        Program::read_vertex(&a).unwrap().chunk(b"ISGN"),
        Program::read_vertex(&b).unwrap().chunk(b"ISGN")
    );
}

#[test]
fn shader_rejects_unknown_decoder_or_affine_addressing() {
    let mut changed = stock(0, 0);
    changed.instructions[4].args[2] = lit(65535.0);
    changed.instructions[4] = changed.instructions[4]
        .replace(changed.instructions[4].args.clone())
        .unwrap();
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("bone decoder")
    );
    let mut changed = stock(0, 0);
    changed.instructions[8].args[1].0[2] = 9;
    changed.instructions[8] = changed.instructions[8]
        .replace(changed.instructions[8].args.clone())
        .unwrap();
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("addressing")
    );
}

#[test]
fn shader_rejects_reused_palette_indices_and_ambiguous_decoders() {
    let mut changed = stock(0, 0);
    changed
        .instructions
        .insert(9, ins(54, vec![dst(0, 1), lit(0.0)]));
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("lifetime")
    );
    let mut changed = stock(0, 0);
    changed
        .instructions
        .extend(changed.instructions[4..6].to_vec());
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("bone decoder")
    );
}

#[test]
fn shader_rejects_wrong_signature_control_flow_and_capacity() {
    let mut changed = stock(0, 0);
    changed
        .instructions
        .insert(8, ins(31 | (1 << 18), vec![temp(0, 0)]));
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("conditional")
    );
    let mut changed = stock(0, 0);
    changed.instructions[3].words[1] = 4078;
    assert!(transform_vertex(&changed.emit().unwrap(), contract()).is_err());
    let source = stock(0, 0).emit().unwrap();
    assert!(
        transform_vertex(
            &source,
            VertexContract {
                bone_palette: 8,
                ..contract()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("cover the rig")
    );
    let mut signature = signature(0);
    signature[28] = 7;
    let changed = Program::synthetic_vertex(
        stock(0, 0).instructions,
        signature,
        super::tests::signature(0),
    );
    assert!(
        transform_vertex(&changed.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("float4")
    );
}

#[test]
fn shader_rejects_invalid_contract_and_corrupt_container() {
    let source = stock(0, 0).emit().unwrap();
    for separation in [f32::NAN, f32::INFINITY, 0.0, -0.1, 1.1] {
        assert!(
            transform_vertex(
                &source,
                VertexContract {
                    separation,
                    ..contract()
                }
            )
            .is_err()
        );
    }
    for marker_bone in [0, 4, 65535] {
        assert!(
            transform_vertex(
                &source,
                VertexContract {
                    marker_bone,
                    ..contract()
                }
            )
            .is_err()
        );
    }
    let mut corrupt = source.clone();
    corrupt[4] ^= 1;
    assert!(transform_vertex(&corrupt, contract()).is_err());
    assert!(transform_vertex(&source[..source.len() - 1], contract()).is_err());
}

fn effect_binding(scale: f32) -> EffectBinding {
    EffectBinding {
        quantization_bits: [0.1f32, -0.2, 0.3, scale].map(f32::to_bits),
        contract: contract(),
    }
}

#[test]
fn effect_bindings_are_bounded_unique_and_sorted_for_reproducibility() {
    let source = stock(0, 0).emit().unwrap();
    let a = effect_binding(0.5);
    let b = effect_binding(0.75);
    let output = transform_effect_vertex(&source, &[a, b]).unwrap();
    assert_eq!(output, transform_effect_vertex(&source, &[b, a]).unwrap());
    assert!(transform_effect_vertex(&source, &[]).is_err());
    assert!(transform_effect_vertex(&source, &[a, a]).is_err());
    assert!(transform_effect_vertex(&source, &[a; 4097]).is_err());
    assert!(transform_effect_vertex(&source, &[effect_binding(0.0)]).is_err());
    assert!(transform_effect_vertex(&source, &[effect_binding(f32::NAN)]).is_err());
    let parsed = Program::read_vertex(&output).unwrap();
    assert_eq!(parsed.instructions[3].words[1], 24);
    assert_eq!(
        parsed.chunk(b"ISGN"),
        Program::read_vertex(&source).unwrap().chunk(b"ISGN")
    );
}

#[test]
fn large_effect_dispatch_groups_complete_contracts_and_bounds_serialized_growth() {
    let source = stock(0, 0).emit().unwrap();
    let mut bindings = (0..4096)
        .map(|index| effect_binding(0.5 + index as f32 / 8192.0))
        .collect::<Vec<_>>();
    let output = transform_effect_vertex(&source, &bindings).unwrap();
    assert_eq!(effects::groups(&bindings).len(), 1);
    assert!(output.len() < 1024 * 1024);
    bindings.reverse();
    assert_eq!(output, transform_effect_vertex(&source, &bindings).unwrap());
    bindings[0].contract.marker_bone = 1;
    bindings[1].contract.bone_palette = 8;
    bindings[2].contract.separation = 0.05;
    assert_eq!(effects::groups(&bindings).len(), 4);
    assert!(transform_effect_vertex(&source, &bindings).is_ok());
    for (index, binding) in bindings.iter_mut().enumerate() {
        binding.contract.separation = 0.05 + index as f32 / 8192.0;
    }
    assert!(
        transform_effect_vertex(&source, &bindings)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}

#[test]
fn effect_control_scope_must_contain_the_decoder_and_every_palette_read() {
    let mut supported = stock(0, 0);
    supported
        .instructions
        .insert(4, ins(31 | (1 << 18), vec![temp(0, 0)]));
    let end = supported.instructions.len() - 1;
    supported.instructions.insert(end, ins(21, vec![]));
    assert!(transform_effect_vertex(&supported.emit().unwrap(), &[effect_binding(0.5)]).is_ok());
    let mut escaped = stock(0, 0);
    escaped
        .instructions
        .insert(4, ins(31 | (1 << 18), vec![temp(0, 0)]));
    escaped.instructions.insert(9, ins(21, vec![]));
    assert!(
        transform_effect_vertex(&escaped.emit().unwrap(), &[effect_binding(0.5)])
            .unwrap_err()
            .to_string()
            .contains("lifetime")
    );
    let mut unmatched = stock(0, 0);
    unmatched.instructions.insert(4, ins(18, vec![]));
    assert!(transform_effect_vertex(&unmatched.emit().unwrap(), &[effect_binding(0.5)]).is_err());
}

#[test]
fn additional_bone_lane_or_direct_palette_consumers_need_an_adapter() {
    let mut source = stock(0, 0);
    source
        .instructions
        .insert(11, ins(54, vec![dst(2, 1), input(0, 0xff)]));
    assert!(
        transform_vertex(&source.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("additional consumer")
    );
    let mut source = stock(0, 0);
    source
        .instructions
        .insert(11, ins(54, vec![dst(2, 15), cb(11, 17, 0xe4)]));
    assert!(
        transform_vertex(&source.emit().unwrap(), contract())
            .unwrap_err()
            .to_string()
            .contains("direct bone row")
    );
}
