//! Generated affine palette rows; these instructions encode the companion's single-marker ABI.
use super::*;

const MARKER_BASE: f32 = 1.0 + 18.0 / 512.0;

fn uint(value: u32) -> Operand {
    Operand(vec![1 | (4 << 12), value])
}

fn scalar(register: u32, lane: u32) -> Operand {
    Scalar { register, lane }.source()
}

fn cb_scalar(row: u32, lane: u32) -> Operand {
    let mut operand = cb(11, row, 0);
    operand.0[0] = (operand.0[0] & !0xfff) | 2 | (2 << 2) | (lane << 4);
    operand
}

fn dynamic_row(index: Scalar, row: u32) -> Operand {
    let mut words = vec![
        2 | (1 << 2) | (0xe4 << 4) | (8 << 12) | (2 << 20) | (3 << 25),
        11,
        row,
    ];
    words.extend(index.source().0);
    Operand(words)
}

fn absolute(mut operand: Operand) -> Operand {
    operand.0[0] |= 1 << 31;
    operand.0.insert(1, 0x81);
    operand
}

fn if_nonzero(condition: Operand) -> Instruction {
    ins(31 | (1 << 18), vec![condition])
}

fn mask_decode(base: u32, marker: u32, output: &mut Vec<Instruction>) {
    let state = base + 15;
    let decoded = base + 16;
    let scratch = base + 17;
    output.extend([
        ins(16, vec![dst(decoded, 1), cb(11, 8, 0x24), cb(11, 8, 0x24)]),
        ins(
            16,
            vec![dst(decoded, 2), cb(11, marker, 0x24), cb(11, marker, 0x24)],
        ),
        ins(52, vec![dst(decoded, 1), scalar(decoded, 0), lit(1e-6)]),
        ins(
            14,
            vec![dst(decoded, 2), scalar(decoded, 1), scalar(decoded, 0)],
        ),
        ins(75, vec![dst(decoded, 2), scalar(decoded, 1)]),
        ins(
            0,
            vec![dst(decoded, 4), scalar(decoded, 1), lit(-MARKER_BASE)],
        ),
        ins(
            50,
            vec![dst(decoded, 4), scalar(decoded, 2), lit(512.0), lit(0.5)],
        ),
        ins(28, vec![dst(decoded, 4), scalar(decoded, 2)]),
        ins(86, vec![dst(decoded, 8), scalar(decoded, 2)]),
        ins(
            50,
            vec![
                dst(decoded, 8),
                scalar(decoded, 3),
                lit(1.0 / 512.0),
                lit(MARKER_BASE),
            ],
        ),
        ins(
            0,
            vec![dst(scratch, 4), scalar(decoded, 1), neg(scalar(decoded, 3))],
        ),
        ins(
            49,
            vec![dst(state, 1), absolute(scalar(scratch, 2)), lit(1e-4)],
        ),
        ins(79, vec![dst(scratch, 4), scalar(decoded, 2), uint(4)]),
        ins(1, vec![dst(state, 1), scalar(state, 0), scalar(scratch, 2)]),
        ins(14, vec![dst(state, 8), lit(1.0), scalar(decoded, 3)]),
        ins(1, vec![dst(scratch, 1), scalar(decoded, 2), uint(1)]),
        ins(1, vec![dst(scratch, 2), scalar(decoded, 2), uint(2)]),
        ins(
            55,
            vec![
                dst(state, 2),
                scalar(base, 0),
                scalar(scratch, 0),
                scalar(scratch, 1),
            ],
        ),
        ins(32, vec![dst(scratch, 1), scalar(state, 1), uint(0)]),
        ins(
            1,
            vec![dst(scratch, 1), scalar(scratch, 0), scalar(state, 0)],
        ),
        if_nonzero(scalar(scratch, 0)),
    ]);
    for axis in 0..3 {
        output.push(ins(
            54,
            vec![dst(base + 1 + axis, 15), cb(11, 8 + axis, 0xe4)],
        ));
    }
    output.push(ins(21, vec![]));
}

fn inverse_pivot(base: u32, separation: f32, output: &mut Vec<Instruction>) {
    output.push(if_nonzero(scalar(base, 0)));
    for axis in 0..3 {
        output.extend([
            ins(
                0,
                vec![
                    dst(base, 4),
                    cb_scalar(8 + axis, 1),
                    neg(scalar(base + 1 + axis, 1)),
                ],
            ),
            ins(
                50,
                vec![
                    dst(base + 1 + axis, 8),
                    scalar(base, 2),
                    lit(separation),
                    scalar(base + 1 + axis, 3),
                ],
            ),
        ]);
    }
    output.push(ins(21, vec![]));
}

fn left_transform(base: u32, marker: u32, output: &mut Vec<Instruction>) {
    // R = marker/rootScale * inverse(nativeRoot), with the marker scale signal removed.
    // The native root is rigid apart from a uniform scale; its inverse linear part is R^T/s^2.
    let inverse = base + 4;
    let state = base + 15;
    output.extend([
        ins(16, vec![dst(base, 2), cb(11, 8, 0x24), cb(11, 8, 0x24)]),
        ins(
            16,
            vec![dst(base, 4), cb(11, marker, 0x24), cb(11, marker, 0x24)],
        ),
        ins(52, vec![dst(inverse, 1), scalar(base, 1), lit(1e-6)]),
        ins(14, vec![dst(inverse, 1), lit(1.0), scalar(inverse, 0)]),
        ins(1, vec![dst(base, 4), scalar(state, 0), scalar(base, 0)]),
        if_nonzero(scalar(base, 2)),
    ]);
    for axis in 0..3 {
        output.push(ins(
            54,
            vec![dst(base + 5 + axis, 15), cb(11, marker + axis, 0xe4)],
        ));
    }
    for axis in 0..3 {
        let marker_row = base + 5 + axis;
        let relative_row = base + 8 + axis;
        output.push(ins(
            56,
            vec![
                dst(marker_row, 7),
                temp(marker_row, 0x24),
                temp(state, 0xff),
            ],
        ));
        for column in 0..3 {
            output.push(ins(
                16,
                vec![
                    dst(relative_row, 1 << column),
                    temp(marker_row, 0x24),
                    cb(11, 8 + column, 0x24),
                ],
            ));
        }
        output.push(ins(
            56,
            vec![
                dst(relative_row, 7),
                temp(relative_row, 0x24),
                temp(inverse, 0),
            ],
        ));
    }
    for axis in 0..3 {
        output.push(ins(
            54,
            vec![dst(inverse, 1 << axis), cb_scalar(8 + axis, 3)],
        ));
    }
    for axis in 0..3 {
        let relative_row = base + 8 + axis;
        let result = base + 11 + axis;
        output.extend([
            ins(
                16,
                vec![
                    dst(relative_row, 8),
                    temp(relative_row, 0x24),
                    temp(inverse, 0x24),
                ],
            ),
            ins(
                0,
                vec![
                    dst(relative_row, 8),
                    scalar(base + 5 + axis, 3),
                    neg(scalar(relative_row, 3)),
                ],
            ),
            ins(
                56,
                vec![dst(result, 15), temp(relative_row, 0), temp(base + 1, 0xe4)],
            ),
            ins(
                50,
                vec![
                    dst(result, 15),
                    temp(relative_row, 0x55),
                    temp(base + 2, 0xe4),
                    temp(result, 0xe4),
                ],
            ),
            ins(
                50,
                vec![
                    dst(result, 15),
                    temp(relative_row, 0xaa),
                    temp(base + 3, 0xe4),
                    temp(result, 0xe4),
                ],
            ),
            ins(
                0,
                vec![dst(result, 8), scalar(result, 3), scalar(relative_row, 3)],
            ),
        ]);
    }
    for axis in 0..3 {
        output.push(ins(
            54,
            vec![dst(base + 1 + axis, 15), temp(base + 11 + axis, 0xe4)],
        ));
    }
    output.push(ins(21, vec![]));
}

pub(super) fn instructions(base: u32, bone: Scalar, contract: VertexContract) -> Vec<Instruction> {
    let marker = 8 + u32::from(contract.marker_bone) * 3;
    let mut output = vec![
        ins(1, vec![dst(base, 1), bone.source(), uint(0x40)]),
        ins(1, vec![bone.destination(), bone.source(), uint(0x3f)]),
        ins(
            38,
            vec![
                Operand(vec![13 << 12]),
                dst(base, 2),
                bone.source(),
                uint(3),
            ],
        ),
    ];
    for axis in 0..3 {
        output.push(ins(
            54,
            vec![
                dst(base + 1 + axis, 15),
                dynamic_row(
                    Scalar {
                        register: base,
                        lane: 1,
                    },
                    8 + axis,
                ),
            ],
        ));
    }
    mask_decode(base, marker, &mut output);
    inverse_pivot(base, contract.separation, &mut output);
    left_transform(base, marker, &mut output);
    output
}
