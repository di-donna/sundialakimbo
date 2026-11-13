//! The native screen-projected reticle is retained once, with its own shader unchanged.
//! Recognize its declared buffers and projection dataflow, never a material or shader tag.
use super::*;

fn signature_register(bytes: Option<&[u8]>, position: bool) -> Option<u32> {
    let bytes = bytes?;
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let count = word(0)? as usize;
    if count > 32 || 8 + count * 24 > bytes.len() {
        return None;
    }
    let mut result = None;
    for index in 0..count {
        let at = 8 + index * 24;
        let name_at = word(at)? as usize;
        let name = bytes.get(name_at..)?.split(|b| *b == 0).next()?;
        let matches = if position {
            name == b"POSITION" && word(at + 4)? == 0 && word(at + 8)? == 0
        } else {
            name == b"SV_POSITION" && word(at + 8)? == 1
        };
        if matches && (word(at + 12)? != 3 || result.replace(word(at + 16)?).is_some()) {
            return None;
        }
    }
    result
}

fn output(register: u32, mask: u32) -> Operand {
    let mut value = dst(register, mask);
    value.0[0] |= 2 << 12;
    value
}

fn same(actual: &Operand, expected: &Operand) -> bool {
    if actual.kind() != expected.kind() {
        return false;
    }
    if actual.kind() == 4 {
        return literal_words(actual) == literal_words(expected);
    }
    actual.indices().is_some()
        && actual.indices() == expected.indices()
        && actual.mask() == expected.mask()
        && actual.lanes() == expected.lanes()
}

fn projection_matches(body: &[Instruction], position: u32) -> bool {
    let Some(tail) = body.len().checked_sub(9).map(|start| &body[start..]) else {
        return false;
    };
    let Some(temporary) = tail[0]
        .args
        .first()
        .and_then(Operand::indices)
        .and_then(|v| v.first())
        .copied()
    else {
        return false;
    };
    let Some(projected) = tail[2]
        .args
        .first()
        .and_then(Operand::indices)
        .and_then(|v| v.first())
        .copied()
    else {
        return false;
    };
    let expected = [
        ins(
            56,
            vec![dst(temporary, 15), temp(projected, 0x55), cb(0, 7, 0xe4)],
        ),
        ins(
            50,
            vec![
                dst(temporary, 15),
                cb(0, 6, 0xe4),
                temp(projected, 0),
                temp(temporary, 0xe4),
            ],
        ),
        ins(
            50,
            vec![
                dst(projected, 15),
                cb(0, 8, 0xe4),
                temp(projected, 0xaa),
                temp(temporary, 0xe4),
            ],
        ),
        ins(
            0,
            vec![dst(projected, 15), temp(projected, 0xe4), cb(0, 9, 0xe4)],
        ),
        ins(14, vec![dst(temporary, 1), lit(1.0), temp(projected, 0xff)]),
        ins(
            50,
            vec![
                output(position, 3),
                temp(projected, 0x04),
                temp(temporary, 0),
                cb(0, 10, 0x04),
            ],
        ),
        ins(
            56,
            vec![dst(projected, 3), temp(projected, 0xae), temp(temporary, 0)],
        ),
        ins(54, vec![output(position, 12), temp(projected, 0x40)]),
        ins(62, vec![]),
    ];
    tail.iter().zip(expected).all(|(actual, expected)| {
        actual.opcode() == expected.opcode()
            && actual.args.len() == expected.args.len()
            && actual
                .args
                .iter()
                .zip(expected.args)
                .all(|(a, b)| same(a, &b))
    })
}

fn reticle_declarations(program: &Program, position: u32, output: u32) -> Option<usize> {
    let first = program.instructions.iter().position(|i| i.opcode() < 88)?;
    let mut buffers = std::collections::BTreeMap::new();
    let mut temps = None;
    let mut position_input = 0;
    let mut position_output = 0;
    for instruction in &program.instructions[..first] {
        match instruction.opcode() {
            89 if instruction.words.len() == 4 && instruction.words[0] & 0x800 == 0 => {
                if buffers
                    .insert(instruction.words[2], instruction.words[3])
                    .is_some()
                {
                    return None;
                }
            }
            104 if instruction.words.len() == 2 => {
                if temps.replace(instruction.words[1]).is_some() {
                    return None;
                }
            }
            95 if instruction.words.len() == 3 => {
                let operand = Operand(instruction.words[1..].to_vec());
                if operand.register(1, position) {
                    if operand.mask() != Some(7) {
                        return None;
                    }
                    position_input += 1;
                }
            }
            103 if instruction.words.len() == 4 && instruction.words[3] == 1 => {
                let operand = Operand(instruction.words[1..3].to_vec());
                if !operand.register(2, output) || operand.mask() != Some(15) {
                    return None;
                }
                position_output += 1;
            }
            101 | 106 => {}
            _ => return None,
        }
    }
    if buffers != std::collections::BTreeMap::from([(0, 12), (11, 7), (12, 8)])
        || !temps.is_some_and(|count| (1..=64).contains(&count))
        || position_input != 1
        || position_output != 1
    {
        return None;
    }
    Some(first)
}

fn shape(program: &Program) -> Option<()> {
    let position = signature_register(program.chunk(b"ISGN"), true)?;
    let output = signature_register(program.chunk(b"OSGN"), false)?;
    let first = reticle_declarations(program, position, output)?;
    let body = &program.instructions[first..];
    if !(10..=256).contains(&body.len()) || !projection_matches(body, output) {
        return None;
    }
    let mut position_reads = 0;
    for (index, instruction) in body.iter().enumerate() {
        if !matches!(instruction.opcode(), 0 | 14 | 16 | 50 | 54 | 56 | 62 | 68)
            || instruction.words[0] & 0x00ff_f800 != 0
            || (instruction.opcode() == 62 && index + 1 != body.len())
        {
            return None;
        }
        for operand in instruction.args.iter().skip(1) {
            if operand.kind() == 1 && operand.indices().is_none() {
                return None;
            }
            if operand.kind() == 8
                && ![(0, 6..12), (11, 5..7), (12, 4..8)]
                    .iter()
                    .any(|(slot, rows)| rows.clone().any(|row| operand.cb(*slot, row)))
            {
                return None;
            }
            if operand.kind() == 1 && operand.register(1, position) {
                position_reads += 1;
                if instruction.opcode() != 50
                    || instruction.args.len() != 4
                    || instruction.args[0].mask() != Some(7)
                    || !same(operand, &instruction.args[1])
                    || operand.lanes() != Some([0, 1, 2, 0])
                    || !same(&instruction.args[2], &cb(11, 5, 0xff))
                    || !same(&instruction.args[3], &cb(11, 5, 0x24))
                {
                    return None;
                }
            }
        }
    }
    (position_reads == 1).then_some(())
}

/// Whether this program is the supported unskinned, screen-projected reticle shape.
/// A matching mesh must remain single, keep its material, and still use common requantization.
pub fn retained_reticle(bytes: &[u8]) -> AuthoringResult<bool> {
    Ok(shape(&Program::read_vertex(bytes)?).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(name: &str, system: u32) -> Vec<u8> {
        let mut bytes = vec![0; 32];
        for (offset, value) in [(0, 1u32), (4, 8), (8, 32), (16, system), (20, 3)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[28] = 15;
        bytes[29] = 15;
        bytes.extend(name.as_bytes());
        bytes.push(0);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes
    }

    fn viewport() -> Program {
        let mut position = dst(0, 7);
        position.0[0] |= 1 << 12;
        let mut input = temp(0, 0x24);
        input.0[0] |= 1 << 12;
        let mut temps = ins(104, vec![]);
        temps.words = vec![(2 << 24) | 104, 2];
        let mut output_decl = ins(103, vec![output(0, 15)]);
        output_decl.words[0] += 1 << 24;
        output_decl.words.push(1);
        let instructions = vec![
            ins(89, vec![cb(0, 12, 0xe4)]),
            ins(89, vec![cb(11, 7, 0xe4)]),
            ins(89, vec![cb(12, 8, 0xe4)]),
            ins(95, vec![position]),
            output_decl,
            temps,
            ins(50, vec![dst(0, 7), input, cb(11, 5, 0xff), cb(11, 5, 0x24)]),
            ins(56, vec![dst(1, 15), temp(0, 0x55), cb(0, 7, 0xe4)]),
            ins(
                50,
                vec![dst(1, 15), cb(0, 6, 0xe4), temp(0, 0), temp(1, 0xe4)],
            ),
            ins(
                50,
                vec![dst(0, 15), cb(0, 8, 0xe4), temp(0, 0xaa), temp(1, 0xe4)],
            ),
            ins(0, vec![dst(0, 15), temp(0, 0xe4), cb(0, 9, 0xe4)]),
            ins(14, vec![dst(1, 1), lit(1.), temp(0, 0xff)]),
            ins(50, vec![output(0, 3), temp(0, 4), temp(1, 0), cb(0, 10, 4)]),
            ins(56, vec![dst(0, 3), temp(0, 0xae), temp(1, 0)]),
            ins(54, vec![output(0, 12), temp(0, 0x40)]),
            ins(62, vec![]),
        ];
        Program::synthetic_vertex(
            instructions,
            signature("POSITION", 0),
            signature("SV_POSITION", 1),
        )
    }

    #[test]
    fn recognizes_projected_reticle_and_rejects_missing_signatures() {
        assert!(shape(&viewport()).is_some());
        let program = Program::synthetic_vertex(vec![ins(62, vec![])], Vec::new(), Vec::new());
        assert!(shape(&program).is_none());
    }

    #[test]
    fn rejects_palette_access_position_w_and_changed_projection() {
        let mut program = viewport();
        program.instructions[1].words[0] |= 0x800;
        assert!(shape(&program).is_none());
        let mut program = viewport();
        let mut position = temp(0, 0xff);
        position.0[0] |= 1 << 12;
        program.instructions[6] = ins(
            50,
            vec![dst(0, 7), position, cb(11, 5, 0xff), cb(11, 5, 0x24)],
        );
        assert!(shape(&program).is_none());
        let mut program = viewport();
        program.instructions[11] = ins(14, vec![dst(1, 1), lit(2.), temp(0, 0xff)]);
        assert!(shape(&program).is_none());
    }
}
