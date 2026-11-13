//! Execute the small SM5 subset used by the synthetic skinning fixture. Expectations below
//! are world-space positions, independent of the generated instruction/register layout.
use super::*;

struct Machine {
    temps: [[u32; 4]; 64],
    input: [u32; 4],
    blend: [u32; 4],
    output: [u32; 4],
    palette: Vec<[u32; 4]>,
}

impl Machine {
    fn read(&self, operand: &Operand) -> [u32; 4] {
        let token = operand.0[0];
        let start = 1 + usize::from(token >> 31 != 0);
        let words = &operand.0[start..];
        let value = match operand.kind() {
            0 => self.temps[words[0] as usize],
            1 => match words[0] {
                0 => self.input,
                1 => self.blend,
                _ => panic!("Unknown synthetic input"),
            },
            4 if token & 3 == 1 => [words[0]; 4],
            4 => words.try_into().unwrap(),
            8 => {
                assert_eq!(words[0], 11);
                let relative = if (token >> 25) & 7 == 3 {
                    self.read(&Operand(words[2..].to_vec()))[0]
                } else {
                    assert_eq!((token >> 25) & 7, 0);
                    0
                };
                self.palette
                    .get((words[1] + relative) as usize)
                    .copied()
                    .unwrap_or_default()
            }
            kind => panic!("Unsupported synthetic operand {kind}"),
        };
        let lanes = operand.lanes().unwrap_or([0, 1, 2, 3]);
        std::array::from_fn(|lane| {
            let value = value[lanes[lane] as usize];
            if start == 1 {
                value
            } else {
                match operand.0[1] {
                    0x41 => (-f32::from_bits(value)).to_bits(),
                    0x81 => f32::from_bits(value).abs().to_bits(),
                    modifier => panic!("Unsupported synthetic modifier {modifier}"),
                }
            }
        })
    }

    fn write(&mut self, operand: &Operand, value: [u32; 4]) {
        let destination = match operand.kind() {
            0 => &mut self.temps[operand.0[1] as usize],
            2 => &mut self.output,
            kind => panic!("Unsupported synthetic destination {kind}"),
        };
        for lane in 0..4 {
            if operand.mask().unwrap() & (1 << lane) != 0 {
                destination[lane] = value[lane];
            }
        }
    }

    fn run(self, bytes: &[u8]) -> [f32; 3] {
        self.run_counted(bytes).0
    }

    fn run_counted(mut self, bytes: &[u8]) -> ([f32; 3], usize, usize) {
        let mut executed = 0;
        let mut fixed_bone_reads = 0;
        let program = Program::read_vertex(bytes).unwrap();
        let mut conditions = vec![true];
        for instruction in program.instructions {
            let opcode = instruction.opcode();
            if opcode >= 88 {
                continue;
            }
            let instruction = instruction.parsed().unwrap();
            let args = &instruction.args;
            let active = *conditions.last().unwrap();
            match opcode {
                31 => {
                    assert_ne!(instruction.words[0] & (1 << 18), 0);
                    conditions.push(active && self.read(&args[0])[0] != 0);
                    continue;
                }
                21 => {
                    conditions.pop().unwrap();
                    assert!(!conditions.is_empty());
                    continue;
                }
                18 => {
                    let previous = conditions.pop().unwrap();
                    let parent = *conditions.last().unwrap();
                    conditions.push(parent && !previous);
                    continue;
                }
                62 => break,
                _ if !active => continue,
                _ => {}
            }
            executed += 1;
            let first_source = if opcode == 38 { 2 } else { 1 };
            fixed_bone_reads += args[first_source..]
                .iter()
                .filter(|operand| {
                    operand.kind() == 8
                        && operand.indices().is_some_and(|indices| {
                            indices.len() == 2 && indices[0] == 11 && indices[1] >= 8
                        })
                })
                .count();
            let source = args[first_source..]
                .iter()
                .map(|operand| self.read(operand))
                .collect::<Vec<_>>();
            let result = std::array::from_fn(|lane| {
                let a = source[0][lane];
                let b = source.get(1).map_or(0, |value| value[lane]);
                let c = source.get(2).map_or(0, |value| value[lane]);
                let (af, bf, cf) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(c));
                let truth = |condition| if condition { u32::MAX } else { 0 };
                match opcode {
                    0 => (af + bf).to_bits(),
                    1 => a & b,
                    14 => (af / bf).to_bits(),
                    16 | 17 => (0..if opcode == 16 { 3 } else { 4 })
                        .map(|component| {
                            f32::from_bits(source[0][component])
                                * f32::from_bits(source[1][component])
                        })
                        .sum::<f32>()
                        .to_bits(),
                    26 => (af - af.floor()).to_bits(),
                    28 => af as u32,
                    32 => truth(a == b),
                    35 => a.wrapping_mul(b).wrapping_add(c),
                    38 => a.wrapping_mul(b),
                    49 => truth(af < bf),
                    50 => (af * bf + cf).to_bits(),
                    51 => af.min(bf).to_bits(),
                    52 => af.max(bf).to_bits(),
                    54 => a,
                    55 => {
                        if a != 0 {
                            b
                        } else {
                            c
                        }
                    }
                    56 => (af * bf).to_bits(),
                    60 => a | b,
                    64 => af.sqrt().recip().to_bits(),
                    75 => af.sqrt().to_bits(),
                    79 => truth(a < b),
                    86 => (a as f32).to_bits(),
                    _ => panic!("Unsupported synthetic opcode {opcode}"),
                }
            });
            self.write(&args[first_source - 1], result);
        }
        assert_eq!(conditions, [true]);
        (
            std::array::from_fn(|lane| f32::from_bits(self.output[lane])),
            executed,
            fixed_bone_reads,
        )
    }
}

fn scene(gap: f32, left: bool, selector: [u32; 4]) -> Machine {
    let mut palette = vec![[0; 4]; 32];
    let signal = 1.0 + 21.0 / 512.0; // Valid marker with both native moving bones active.
    let (sin, cos) = 0.37f32.sin_cos();
    let root = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, -gap / 2.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let bone = [
        [cos, -sin, 0.0, 0.12],
        [sin, cos, 0.0, 0.23 - gap / 2.0],
        [0.0, 0.0, 1.0, 0.34],
    ];
    for axis in 0..3 {
        palette[8 + axis] = root[axis].map(f32::to_bits);
        palette[14 + axis] = bone[axis].map(f32::to_bits);
        palette[17 + axis] = std::array::from_fn(|lane| {
            (root[axis][lane] * if lane < 3 { signal } else { 1.0 }).to_bits()
        });
    }
    palette[5] = selector;
    Machine {
        temps: [[0; 4]; 64],
        // The fixture uses xyzx for homogeneous position, hence x=1. Its bone rotates
        // independently: a missing inverse-pivot correction rotates the gap and fails.
        input: [
            1.0,
            0.1 + if left { gap } else { 0.0 },
            0.2,
            (2 + if left { 0xc0 } else { 0 }) as f32 / 32767.0,
        ]
        .map(f32::to_bits),
        blend: [0; 4],
        output: [0; 4],
        palette,
    }
}

#[test]
fn two_bone_skinning_preserves_each_weight_and_independent_hand_transform() {
    let weighted = transform_two_bone_vertex_with_source_palette(
        &super::weighted::fixture().emit().unwrap(),
        contract(),
        3,
    )
    .unwrap();
    let rigid = transform_vertex(&stock(0, 0).emit().unwrap(), contract()).unwrap();
    for left in [false, true] {
        for weight in [0, 1, 64, 127, 128, 191, 254, 255] {
            let mut state = scene(0.36, left, [0; 4]);
            state.input[3] = ((if left { 192 } else { 0 }) as f32 / 32767.0).to_bits();
            state.blend = [0, 2, weight, 255 - weight];
            let actual = state.run(&weighted);
            let reference = [0, 2].map(|bone| {
                let mut state = scene(0.36, left, [0; 4]);
                state.input[3] = ((bone + if left { 192 } else { 0 }) as f32 / 32767.0).to_bits();
                state.run(&rigid)
            });
            for axis in 0..3 {
                let expected = (reference[0][axis] * weight as f32
                    + reference[1][axis] * (255 - weight) as f32)
                    / 255.0;
                assert!((actual[axis] - expected).abs() < 1e-5);
            }
        }
    }
}

fn check_pair(bytes: &[u8], gap: f32, selector: [u32; 4]) {
    let right = scene(gap, false, selector).run(bytes);
    let left = scene(gap, true, selector).run(bytes);
    let (sin, cos) = 0.37f32.sin_cos();
    let center = [cos - 0.1 * sin + 0.12, sin + 0.1 * cos + 0.23, 0.54];
    for axis in 0..3 {
        let offset = if axis == 1 { gap / 2.0 } else { 0.0 };
        assert!(
            (right[axis] - (center[axis] - offset)).abs() < 1e-5,
            "gap {gap}, right {right:?}, center {center:?}"
        );
        assert!(
            (left[axis] - (center[axis] + offset)).abs() < 1e-5,
            "gap {gap}, left {left:?}, center {center:?}"
        );
    }
}

#[test]
fn adjustable_gap_executes_centered_main_and_copy_vertices() {
    let source = stock(0, 0).emit().unwrap();
    for gap in [0.05, 0.36, 0.8] {
        let output = transform_vertex(
            &source,
            VertexContract {
                separation: gap,
                ..contract()
            },
        )
        .unwrap();
        check_pair(&output, gap, [0; 4]);
    }
}

#[test]
fn visibility_branch_preserves_hidden_output_and_pairs_only_the_active_decoder() {
    let mut source = stock(0, 0);
    let mut output_register = dst(0, 15);
    output_register.0[0] |= 2 << 12;
    source.instructions.splice(
        4..4,
        [
            ins(49, vec![dst(3, 1), input(0, 0), lit(0.0)]),
            ins(31 | (1 << 18), vec![temp(3, 0)]),
            ins(54, vec![output_register, vector([0; 4])]),
            ins(18, vec![]),
        ],
    );
    source
        .instructions
        .insert(source.instructions.len() - 1, ins(21, vec![]));
    let output = transform_vertex(&source.emit().unwrap(), contract()).unwrap();
    check_pair(&output, 0.36, [0; 4]);
    for left in [false, true] {
        let mut hidden = scene(0.36, left, [0; 4]);
        hidden.input[0] = (-1.0f32).to_bits();
        assert_eq!(hidden.run(&output), [0.; 3]);
    }
}

#[test]
fn appended_palette_capacity_preserves_the_source_vertex_bounds() {
    let mut source = stock(0, 0);
    source.instructions[0].words[3] = 17; // Header plus exactly three source affine frames.
    let bytes = source.emit().unwrap();
    assert!(transform_vertex(&bytes, contract()).is_err());
    let output = transform_vertex_with_source_palette(&bytes, contract(), 3).unwrap();
    let program = Program::read_vertex(&output).unwrap();
    assert!(program.instructions[0].words[3] >= 20);
    check_pair(&output, 0.36, [0; 4]);
    // A source model may bind more rows than its program declares. This API requires
    // the caller's independent model/rig/lane proof and grows the output declaration.
    let output = transform_vertex_with_source_palette(&bytes, contract(), 4).unwrap();
    assert!(Program::read_vertex(&output).unwrap().instructions[0].words[3] >= 20);
    check_pair(&output, 0.36, [0; 4]);
    for palette in [0, 5] {
        assert!(transform_vertex_with_source_palette(&bytes, contract(), palette).is_err());
    }
    let mut source = stock(0, 0);
    source.instructions[0].words[3] = 10; // Still requires the header and at least one native frame.
    assert!(transform_vertex_with_source_palette(&source.emit().unwrap(), contract(), 3).is_err());
}

#[test]
fn native_minimum_and_maximum_preserve_clamping_after_pair_transform() {
    let mut source = stock(0, 0);
    // Native rigid shaders can clamp a computed value with MIN/MAX. This remains ordinary
    // straight-line arithmetic; it must retain both operands after palette replacement.
    let floor = [0.99f32, 0.6, 0.4, 0.0];
    let ceiling = [1.0f32, 0.8, 0.5, 0.0];
    source.instructions.insert(
        source.instructions.len() - 2,
        ins(
            52,
            vec![dst(1, 7), temp(1, 0xe4), vector(floor.map(f32::to_bits))],
        ),
    );
    source.instructions.insert(
        source.instructions.len() - 2,
        ins(
            51,
            vec![dst(1, 7), temp(1, 0xe4), vector(ceiling.map(f32::to_bits))],
        ),
    );
    let transformed = transform_vertex(&source.emit().unwrap(), contract()).unwrap();
    let baseline = transform_vertex(&stock(0, 0).emit().unwrap(), contract()).unwrap();
    for left in [false, true] {
        let expected = scene(0.36, left, [0; 4]).run(&baseline);
        let actual = scene(0.36, left, [0; 4]).run(&transformed);
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis].max(floor[axis]).min(ceiling[axis])).abs() < 1e-5
            );
        }
    }
}

#[test]
fn native_fraction_preserves_wrapping_of_negative_and_positive_values() {
    let mut source = stock(0, 0);
    let offset = [-2.25f32, 3.4, -0.5, 0.0];
    let at = source.instructions.len() - 2;
    source.instructions.splice(
        at..at,
        [
            ins(
                0,
                vec![dst(1, 7), temp(1, 0xe4), vector(offset.map(f32::to_bits))],
            ),
            ins(26, vec![dst(1, 7), temp(1, 0xe4)]),
        ],
    );
    let transformed = transform_vertex(&source.emit().unwrap(), contract()).unwrap();
    let baseline = transform_vertex(&stock(0, 0).emit().unwrap(), contract()).unwrap();
    for left in [false, true] {
        let expected = scene(0.36, left, [0; 4]).run(&baseline);
        let actual = scene(0.36, left, [0; 4]).run(&transformed);
        for axis in 0..3 {
            let value = expected[axis] + offset[axis];
            assert!((actual[axis] - (value - value.floor())).abs() < 1e-5);
            assert!((0.0..1.0).contains(&actual[axis]));
        }
    }
}

#[test]
fn native_reciprocal_square_root_preserves_vector_normalization() {
    let mut source = stock(0, 0);
    let at = source.instructions.len() - 2;
    source.instructions.splice(
        at..at,
        [
            ins(16, vec![dst(3, 1), temp(1, 0xe4), temp(1, 0xe4)]),
            ins(64, vec![dst(3, 1), temp(3, 0)]),
            ins(56, vec![dst(1, 7), temp(1, 0xe4), temp(3, 0)]),
        ],
    );
    let transformed = transform_vertex(&source.emit().unwrap(), contract()).unwrap();
    let baseline = transform_vertex(&stock(0, 0).emit().unwrap(), contract()).unwrap();
    for left in [false, true] {
        let expected = scene(0.36, left, [0; 4]).run(&baseline);
        let length = expected.into_iter().map(|v| v * v).sum::<f32>().sqrt();
        let actual = scene(0.36, left, [0; 4]).run(&transformed);
        for axis in 0..3 {
            assert!((actual[axis] - expected[axis] / length).abs() < 1e-5);
        }
    }
}

#[test]
fn shared_effect_branches_execute_each_gap_and_preserve_unmatched_models() {
    let source = stock(0, 0).emit().unwrap();
    let bindings = [0.05, 0.36, 0.8].map(|gap| EffectBinding {
        contract: VertexContract {
            separation: gap,
            ..contract()
        },
        ..effect_binding(gap + 1.0)
    });
    let output = transform_effect_vertex(&source, &bindings).unwrap();
    for binding in bindings {
        check_pair(
            &output,
            binding.contract.separation,
            binding.quantization_bits,
        );
        let mut near_miss = binding.quantization_bits;
        near_miss[0] ^= 1;
        assert_eq!(
            scene(0.0, false, near_miss).run(&output),
            scene(0.0, false, near_miss).run(&source)
        );
    }
}

#[test]
fn large_grouped_effect_dispatch_preserves_every_pin_and_unmatched_native_path() {
    let source = stock(0, 0).emit().unwrap();
    let bindings = (0..193)
        .map(|index| EffectBinding {
            contract: VertexContract {
                separation: [0.05, 0.36, 0.8][index % 3],
                ..contract()
            },
            ..effect_binding(1.0 + index as f32 / 256.0)
        })
        .collect::<Vec<_>>();
    let output = transform_effect_vertex(&source, &bindings).unwrap();
    for binding in bindings {
        check_pair(
            &output,
            binding.contract.separation,
            binding.quantization_bits,
        );
        for lane in 0..4 {
            let mut near_miss = binding.quantization_bits;
            near_miss[lane] ^= 1;
            assert_eq!(
                scene(0.0, false, near_miss).run(&output),
                scene(0.0, false, near_miss).run(&source)
            );
        }
    }
}

#[test]
fn shared_effects_expand_for_source_derived_markers_and_preserve_unmatched_high_bones() {
    let mut source = stock(0, 0);
    source.instructions[0].words[3] = 17;
    let source = source.emit().unwrap();
    let baseline = transform_vertex(&stock(0, 0).emit().unwrap(), contract()).unwrap();
    for marker in [1, 3, 5, 12, 54] {
        let binding = EffectBinding {
            contract: VertexContract {
                marker_bone: marker,
                bone_palette: marker.max(2) + 2,
                ..contract()
            },
            ..effect_binding(0.5)
        };
        let output = transform_effect_vertex(&source, &[binding]).unwrap();
        assert!(
            Program::read_vertex(&output).unwrap().instructions[0].words[3]
                >= 8 + u32::from(binding.contract.bone_palette) * 3
        );
        for left in [false, true] {
            let expected = scene(0.36, left, binding.quantization_bits).run(&baseline);
            let mut machine = scene(0.36, left, binding.quantization_bits);
            let marker_frame = machine.palette[17..20].to_vec();
            machine.palette[17..20].fill([0; 4]);
            machine
                .palette
                .resize(8 + usize::from(binding.contract.bone_palette) * 3, [0; 4]);
            let at = 8 + usize::from(marker) * 3;
            machine.palette[at..at + 3].copy_from_slice(&marker_frame);
            let actual = machine.run(&output);
            for axis in 0..3 {
                assert!((actual[axis] - expected[axis]).abs() < 1e-5);
            }
        }
        let unmatched = || {
            let mut machine = scene(0.0, false, [0; 4]);
            let frame = machine.palette[14..17].to_vec();
            machine.palette.resize(8 + 71 * 3, [0; 4]);
            machine.palette[8 + 70 * 3..8 + 71 * 3].copy_from_slice(&frame);
            machine.input[3] = (70.0f32 / 32767.).to_bits();
            machine
        };
        assert_eq!(unmatched().run(&output), unmatched().run(&source));
    }
}

#[test]
fn balanced_effect_lookup_bounds_unmatched_work_without_reading_private_markers() {
    let source = stock(0, 0).emit().unwrap();
    let bindings = (0..4096u32)
        .map(|index| EffectBinding {
            quantization_bits: [
                (index % 16) as f32,
                ((index / 16) % 16) as f32,
                (index / 256) as f32,
                1.0,
            ]
            .map(f32::to_bits),
            contract: VertexContract {
                separation: 0.05 + (index % 44) as f32 / 100.0,
                ..contract()
            },
        })
        .collect::<Vec<_>>();
    let groups = effects::groups(&bindings);
    assert_eq!(effects::selection::new(&groups).unwrap().depth(), 9);
    let output = transform_effect_vertex(&source, &bindings).unwrap();
    for index in [0, 1, 127, 128, 2047, 2048, 4094, 4095] {
        let binding = bindings[index];
        check_pair(
            &output,
            binding.contract.separation,
            binding.quantization_bits,
        );
        for lane in 0..4 {
            let mut pin = binding.quantization_bits;
            pin[lane] ^= 1;
            let (expected, native_work, _) = scene(0.0, false, pin).run_counted(&source);
            let (actual, work, fixed_reads) = scene(0.0, false, pin).run_counted(&output);
            assert_eq!(actual, expected);
            assert_eq!(fixed_reads, 0, "Unmatched model read private marker rows");
            assert!(
                work <= native_work + 49,
                "Unmatched work scales with selector count: {work}/{native_work}"
            );
        }
    }
}
