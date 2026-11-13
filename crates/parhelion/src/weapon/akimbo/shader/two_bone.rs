//! Preserve native packed two-influence skinning while replacing each affine input independently.
use super::*;
use std::collections::BTreeMap;

const REGISTERS: u32 = 21;

fn input(register: u32, lane: u32) -> Operand {
    Operand(vec![
        2 | (2 << 2) | (lane << 4) | (1 << 12) | (1 << 20),
        register,
    ])
}

fn signature(program: &Program) -> AuthoringResult<(u32, u32, usize)> {
    let bytes = program
        .chunk(b"ISGN")
        .ok_or_else(|| invalid("Akimbo weighted shader has no input signature"))?;
    let word = |at| -> AuthoringResult<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or_else(|| invalid("Truncated weighted signature"))?
                .try_into()
                .unwrap(),
        ))
    };
    let count = word(0)? as usize;
    if count == 0 || count > 32 || 8 + count * 24 > bytes.len() || program.chunk(b"OSGN").is_none()
    {
        return Err(invalid(
            "Akimbo weighted shader has an invalid stage signature",
        ));
    }
    let mut position = None;
    let mut blend = None;
    let mut registers = std::collections::BTreeSet::new();
    for at in (0..count).map(|i| 8 + i * 24) {
        let register = word(at + 16)?;
        if register > 31 || !registers.insert(register) {
            return Err(invalid("Akimbo weighted input registers overlap"));
        }
        let name = bytes
            .get(word(at)? as usize..)
            .and_then(|tail| tail.iter().position(|b| *b == 0).map(|end| &tail[..end]))
            .ok_or_else(|| invalid("Akimbo weighted semantic name is invalid"))?;
        let destination = if name.eq_ignore_ascii_case(b"POSITION") {
            if word(at + 12)? != 3 {
                return Err(invalid("Weighted POSITION0 must be float4"));
            }
            &mut position
        } else if name.eq_ignore_ascii_case(b"BLENDINDICES") {
            if word(at + 12)? != 1 {
                return Err(invalid("Packed BLENDINDICES0 must be uint4"));
            }
            &mut blend
        } else {
            if name.eq_ignore_ascii_case(b"BLENDWEIGHT") {
                return Err(invalid(
                    "Separate or four-influence weights require a separate adapter",
                ));
            }
            continue;
        };
        if word(at + 4)? != 0
            || word(at + 8)? != 0
            || bytes[at + 20] != 15
            || destination.replace((register, at)).is_some()
        {
            return Err(invalid(
                "Akimbo weighted semantic is ambiguous or incomplete",
            ));
        }
    }
    match (position, blend) {
        (Some((position, at)), Some((blend, _))) => Ok((position, blend, at)),
        _ => Err(invalid(
            "Akimbo needs packed two-influence POSITION0/BLENDINDICES0 inputs",
        )),
    }
}

fn addresses(
    instruction: &Instruction,
    blend: u32,
) -> AuthoringResult<Vec<(Scalar, (usize, usize))>> {
    let args = &instruction.args;
    let (destination, source, factor, offset) = match instruction.opcode() {
        38 if args.len() == 4 && args[0].kind() == 13 && args[2].register(1, blend) => {
            (&args[1], &args[2], &args[3], [0; 4])
        }
        35 if args.len() == 4 && args[2].register(1, blend) => {
            let offsets = literal_words(&args[3])
                .ok_or_else(|| invalid("Weighted palette index offsets must be literal"))?;
            (&args[0], &args[2], &args[1], offsets)
        }
        _ => return Ok(Vec::new()),
    };
    if instruction.words[0] & 0x00ff_f800 != 0 || destination.kind() != 0 {
        return Err(invalid("Weighted palette index modifiers are unsupported"));
    }
    let [register] = destination
        .indices()
        .ok_or_else(|| invalid("Weighted palette index destination is indirect"))?
    else {
        return Err(invalid(
            "Weighted palette index destination is not scalar-addressed",
        ));
    };
    let mask = destination
        .mask()
        .ok_or_else(|| invalid("Weighted palette index destination has no mask"))?;
    let lanes = source
        .lanes()
        .ok_or_else(|| invalid("Weighted bone selector has no component selection"))?;
    let factor =
        literal_words(factor).ok_or_else(|| invalid("Weighted palette stride must be literal"))?;
    let mut result = Vec::new();
    for lane in 0..4 {
        if mask & (1 << lane) != 0 {
            if lanes[lane] > 1 || factor[lane] != 3 || offset[lane] > 2 {
                return Err(invalid(
                    "Akimbo weighted palette needs two bones and three affine rows",
                ));
            }
            result.push((
                Scalar {
                    register: *register,
                    lane: lane as u32,
                },
                (lanes[lane] as usize, offset[lane] as usize),
            ));
        }
    }
    Ok(result)
}

fn rows(
    base: u32,
    position: u32,
    blend: u32,
    influence: u32,
    contract: VertexContract,
    effects: Option<(Scalar, &[Vec<EffectBinding>])>,
) -> Vec<Instruction> {
    let bone = Scalar {
        register: base + 20,
        lane: 0,
    };
    let flags = Scalar {
        register: base + 20,
        lane: 1,
    };
    let mut whole_position = temp(position, 0xe4);
    whole_position.0[0] |= 1 << 12;
    let mut result = vec![
        ins(54, vec![dst(base + 14, 15), whole_position]),
        ins(54, vec![bone.destination(), input(blend, influence)]),
        ins(
            50,
            vec![
                flags.destination(),
                input(position, 3),
                lit(32767.0),
                lit(0.1),
            ],
        ),
        ins(28, vec![flags.destination(), flags.source()]),
        ins(
            1,
            vec![
                flags.destination(),
                flags.source(),
                Operand(vec![1 | (4 << 12), 0x40]),
            ],
        ),
    ];
    let combine = || ins(60, vec![bone.destination(), bone.source(), flags.source()]);
    if let Some((selector, groups)) = effects {
        result.extend(super::effects::original_rows(base, bone));
        result.push(combine());
        result.extend(super::effects::generated_rows(base, bone, selector, groups));
    } else {
        result.push(combine());
        result.extend(single_marker::instructions(base, bone, contract));
    }
    result
}

pub(super) fn transform(
    bytes: &[u8],
    contract: VertexContract,
    source_palette: u16,
    effects: Option<&[EffectBinding]>,
) -> AuthoringResult<Vec<u8>> {
    validate_contract(contract)?;
    if source_palette == 0 || source_palette > contract.bone_palette {
        return Err(invalid("Weighted source palette exceeds its generated rig"));
    }
    let mut program = Program::read_vertex(bytes)?;
    let (position, blend, signature_row) = signature(&program)?;
    let (first_body, temps_declaration, temps, cb_declaration, cb_rows) = declarations(&program)?;
    if temps > 4096 - 2 * REGISTERS {
        return Err(invalid(
            "Weighted shader has insufficient temporary capacity",
        ));
    }
    let scopes = control_scopes(&program, true)?;
    let mut indexed = BTreeMap::<(u32, u32), (usize, usize, usize)>::new();
    let mut reads = [[0; 3]; 2];
    let mut body = program.instructions.clone();
    let mut position_declarations = 0;
    let mut blend_declarations = 0;
    for instruction in &mut body[..first_body] {
        if instruction.opcode() != 95 {
            continue;
        }
        let parsed = instruction.parsed()?;
        if parsed.args.len() != 1 {
            return Err(invalid("Weighted input declaration is ambiguous"));
        }
        let operand = &parsed.args[0];
        if operand.register(1, position) {
            if !matches!(operand.mask(), Some(7 | 15)) {
                return Err(invalid("Weighted POSITION0 declaration differs"));
            }
            position_declarations += 1;
            let mut expanded = operand.clone();
            expanded.0[0] = (expanded.0[0] & !0xF0) | 0xF0;
            *instruction = parsed.replace(vec![expanded])?;
        }
        if operand.register(1, blend) {
            if operand.mask() != Some(15) {
                return Err(invalid("Weighted BLENDINDICES0 declaration differs"));
            }
            blend_declarations += 1;
        }
    }
    if position_declarations != 1 || blend_declarations != 1 {
        return Err(invalid("Weighted inputs must be declared exactly once"));
    }
    for index in first_body..body.len() {
        let original = &program.instructions[index];
        if ![
            0, 14, 16, 17, 18, 21, 26, 28, 31, 35, 38, 45, 49, 50, 51, 52, 54, 56, 62, 64, 68, 86,
        ]
        .contains(&original.opcode())
            || original.opcode() == 62 && index + 1 != body.len()
        {
            return Err(invalid(format!(
                "Weighted shader operation {} at {index} is unsupported",
                original.opcode()
            )));
        }
        let added = addresses(original, blend)?;
        let mut args = original.args.clone();
        for operand in &mut args {
            if let Some(relative) = dynamic_palette(operand)? {
                let &(bone, axis, producer) = indexed
                    .get(&(relative.register, relative.lane))
                    .ok_or_else(|| invalid("Weighted palette read has no proven live index"))?;
                if scopes[index] != scopes[producer] {
                    return Err(invalid("Weighted palette read leaves its index scope"));
                }
                *operand = redirected(operand, temps + bone as u32 * REGISTERS + 1 + axis as u32);
                reads[bone][axis] += 1;
            } else {
                if operand.register(1, position)
                    && operand.lanes().is_none_or(|lanes| lanes.contains(&3))
                {
                    return Err(invalid("Weighted source uses the private copy-state lane"));
                }
                validate_direct_operand(operand, temps)?;
            }
        }
        indexed.retain(|&(register, lane), _| !writes(original, Scalar { register, lane }));
        for (address, (bone, axis)) in added {
            indexed.insert((address.register, address.lane), (bone, axis, index));
        }
        body[index] = original.replace(args)?;
    }
    if reads.into_iter().flatten().any(|count| count == 0) {
        return Err(invalid(
            "Weighted shader does not consume both complete affine frames",
        ));
    }
    body[temps_declaration].words[1] = temps + 2 * REGISTERS;
    let palette = effects.map_or(contract.bone_palette, |bindings| {
        bindings
            .iter()
            .map(|b| b.contract.bone_palette)
            .max()
            .unwrap_or(contract.bone_palette)
    });
    body[cb_declaration].words[3] = cb_rows.max(32).max(8 + u32::from(palette) * 3);
    let groups = effects.map(super::effects::groups);
    let selection = groups
        .as_deref()
        .map(super::effects::selection::new)
        .transpose()?;
    let selector = Scalar {
        register: temps + 19,
        lane: 0,
    };
    let mut output = Vec::new();
    for (index, instruction) in body.into_iter().enumerate() {
        if index == first_body {
            if let Some(selection) = &selection {
                selection.instructions(temps, &mut output);
            }
            for influence in 0..2 {
                output.extend(rows(
                    temps + influence * REGISTERS,
                    position,
                    blend,
                    influence,
                    contract,
                    groups.as_deref().map(|groups| (selector, groups)),
                ));
            }
        }
        output.push(instruction);
    }
    program.instructions = output;
    program.chunk_mut(b"ISGN").unwrap()[signature_row + 21] = 15;
    if effects.is_some() {
        return super::effects::emit(program);
    }
    let output = program.emit()?;
    Program::read_vertex(&output)?;
    Ok(output)
}
