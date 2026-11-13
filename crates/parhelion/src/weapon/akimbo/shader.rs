//! Stock-to-private vertex-program transformation for rigid, single-marker pairs.
//!
//! Recognizes the native index decode and three affine palette-row reads structurally. The
//! donor's lighting, UV, projection and stage signatures remain its own. Only palette operands
//! are redirected to generated rows. No shader from another weapon is used as a template.

use crate::dxbc::{Instruction, Operand, Program, cb, dst, ins, lit, neg, temp};
use crate::{AuthoringResult, error::invalid};

#[path = "shader/effects.rs"]
mod effects;
#[path = "shader/reticle.rs"]
mod reticle;
#[path = "shader/single_marker.rs"]
mod single_marker;
#[path = "shader/two_bone.rs"]
mod two_bone;
pub use effects::{EffectBinding, transform_effect_vertex};
pub use reticle::retained_reticle;

pub const SINGLE_MARKER_SHADER_REVISION: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct VertexContract {
    pub marker_bone: u16,
    pub bone_palette: u16,
    /// Must equal both the geometry's baked left-copy translation on weapon-local Y and the
    /// native profile's gap. The native pose supplies the common centering shift (-gap/2).
    pub separation: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Scalar {
    register: u32,
    lane: u32,
}

impl Scalar {
    fn source(self) -> Operand {
        // Select-one rather than a broadcast swizzle, matching native scalar operands.
        Operand(vec![
            2 | (2 << 2) | (self.lane << 4) | (1 << 20),
            self.register,
        ])
    }

    fn destination(self) -> Operand {
        dst(self.register, 1 << self.lane)
    }
}

struct Layout {
    first_body: usize,
    decode: usize,
    input: u32,
    bone: Scalar,
    rows: [Scalar; 3],
    temps_declaration: usize,
    cb_declaration: usize,
    temps: u32,
    cb_rows: u32,
}

fn scalar_source(operand: &Operand) -> Option<Scalar> {
    let &[register] = operand.indices()? else {
        return None;
    };
    let lanes = operand.lanes()?;
    (operand.kind() == 0 && lanes.iter().all(|lane| *lane == lanes[0])).then_some(Scalar {
        register,
        lane: lanes[0],
    })
}

fn scalar_destination(operand: &Operand) -> Option<Scalar> {
    let &[register] = operand.indices()? else {
        return None;
    };
    let mask = operand.mask()?;
    (operand.kind() == 0 && mask.count_ones() == 1).then_some(Scalar {
        register,
        lane: mask.trailing_zeros(),
    })
}

fn literal_words(operand: &Operand) -> Option<[u32; 4]> {
    if operand.kind() != 4 {
        return None;
    }
    match operand.0.as_slice() {
        [token, scalar] if token & 3 == 1 => Some([*scalar; 4]),
        [token, x, y, z, w] if token & 3 == 2 => Some([*x, *y, *z, *w]),
        _ => None,
    }
}

fn decode_candidate(mad: &Instruction, convert: &Instruction) -> Option<(u32, Scalar)> {
    if mad.opcode() != 50
        || convert.opcode() != 28
        || mad.args.len() != 4
        || convert.args.len() != 2
    {
        return None;
    }
    let bone = scalar_destination(&mad.args[0])?;
    let &[input] = mad.args[1].indices()? else {
        return None;
    };
    (mad.words[0] & 0x00ff_f800 == 0
        && convert.words[0] & 0x00ff_f800 == 0
        && mad.args[1].kind() == 1
        && mad.args[1].lanes()? == [3; 4]
        && literal_words(&mad.args[2])? == [32767.0f32.to_bits(); 4]
        && literal_words(&mad.args[3])? == [0.1f32.to_bits(); 4]
        && scalar_destination(&convert.args[0])? == bone
        && scalar_source(&convert.args[1])? == bone)
        .then_some((input, bone))
}

fn index_rows(instructions: &[Instruction], bone: Scalar) -> AuthoringResult<[Scalar; 3]> {
    let mut rows = [None; 3];
    let mut bone_available = true;
    for instruction in instructions {
        let args = &instruction.args;
        if !bone_available || args.len() != 4 || instruction.words[0] & 0x00ff_f800 != 0 {
            return Err(invalid(
                "Akimbo shader: unsupported palette index calculation",
            ));
        }
        match instruction.opcode() {
            38 if args[0].kind() == 13
                && scalar_source(&args[2]) == Some(bone)
                && literal_words(&args[3]) == Some([3; 4]) =>
            {
                let row = scalar_destination(&args[1])
                    .ok_or_else(|| invalid("Akimbo shader: palette index is not scalar"))?;
                if rows[0].replace(row).is_some() {
                    return Err(invalid("Akimbo shader: repeated palette index"));
                }
                bone_available &= row != bone;
            }
            35 if scalar_source(&args[2]) == Some(bone) => {
                let &[register] = args[0]
                    .indices()
                    .ok_or_else(|| invalid("Akimbo shader: palette row register"))?
                else {
                    return Err(invalid("Akimbo shader: palette row register"));
                };
                let mask = args[0]
                    .mask()
                    .filter(|mask| mask.count_ones() == 2)
                    .ok_or_else(|| invalid("Akimbo shader: expected two affine row indices"))?;
                if args[0].kind() != 0 {
                    return Err(invalid("Akimbo shader: palette row destination"));
                }
                let factors = literal_words(&args[1])
                    .ok_or_else(|| invalid("Akimbo shader: palette row factors"))?;
                let offsets = literal_words(&args[3])
                    .ok_or_else(|| invalid("Akimbo shader: palette row offsets"))?;
                for lane in 0..4 {
                    if mask & (1 << lane) == 0 {
                        continue;
                    }
                    let offset = offsets[lane as usize];
                    if factors[lane as usize] != 3
                        || !(1..=2).contains(&offset)
                        || rows[offset as usize]
                            .replace(Scalar { register, lane })
                            .is_some()
                    {
                        return Err(invalid("Akimbo shader: unexpected affine row offset"));
                    }
                    bone_available &= (Scalar { register, lane }) != bone;
                }
            }
            _ => {
                return Err(invalid(
                    "Akimbo shader: unsupported palette index calculation",
                ));
            }
        }
    }
    let [Some(x), Some(y), Some(z)] = rows else {
        return Err(invalid("Akimbo shader: missing affine row index"));
    };
    if x == y || x == z || y == z {
        return Err(invalid("Akimbo shader: aliased affine row indices"));
    }
    Ok([x, y, z])
}

fn declarations(program: &Program) -> AuthoringResult<(usize, usize, u32, usize, u32)> {
    let mut temps = None;
    let mut cb11 = None;
    let first = program
        .instructions
        .iter()
        .position(|instruction| instruction.opcode() < 88)
        .ok_or_else(|| invalid("Akimbo shader: no executable instructions"))?;
    for (index, instruction) in program.instructions[..first].iter().enumerate() {
        match instruction.opcode() {
            104 if instruction.words.len() == 2 => {
                if temps.replace((index, instruction.words[1])).is_some() {
                    return Err(invalid("Akimbo shader: repeated temporary declaration"));
                }
            }
            89 if instruction.words.len() == 4 && instruction.words[2] == 11 => {
                if instruction.words[0] & 0x800 == 0
                    || cb11.replace((index, instruction.words[3])).is_some()
                {
                    return Err(invalid(
                        "Akimbo shader: expected one dynamic palette buffer",
                    ));
                }
            }
            88 | 89 | 95 | 101 | 103 | 106 => {}
            _ => return Err(invalid("Akimbo shader: unsupported vertex declaration")),
        }
    }
    let (temps_index, temps) =
        temps.ok_or_else(|| invalid("Akimbo shader: missing temporary declaration"))?;
    let (cb_index, cb_rows) =
        cb11.ok_or_else(|| invalid("Akimbo shader: missing palette buffer"))?;
    if temps == 0 || temps > 4077 || !(11..=200).contains(&cb_rows) {
        return Err(invalid(
            "Akimbo shader: temporary or palette capacity is outside the adapter limits",
        ));
    }
    Ok((first, temps_index, temps, cb_index, cb_rows))
}

fn validate_signature(program: &Program, input: u32) -> AuthoringResult<()> {
    let bytes = program
        .chunk(b"ISGN")
        .ok_or_else(|| invalid("Akimbo shader: unsupported input signature"))?;
    let read = |at: usize| -> AuthoringResult<u32> {
        let value = bytes
            .get(at..at + 4)
            .ok_or_else(|| invalid("Akimbo shader: truncated input signature"))?;
        Ok(u32::from_le_bytes(value.try_into().unwrap()))
    };
    let count = read(0)? as usize;
    if count == 0 || count > 32 || 8 + count * 24 > bytes.len() {
        return Err(invalid("Akimbo shader: invalid input signature table"));
    }
    let mut found = 0;
    for index in 0..count {
        let at = 8 + index * 24;
        if read(at + 16)? == input {
            let name_at = read(at)? as usize;
            let name = bytes.get(name_at..).and_then(|tail| {
                tail.iter()
                    .position(|byte| *byte == 0)
                    .map(|end| &tail[..end])
            });
            if name.is_none_or(|name| !name.eq_ignore_ascii_case(b"POSITION"))
                || read(at + 4)? != 0
                || read(at + 8)? != 0
            {
                return Err(invalid(
                    "Akimbo shader: rigid decoder does not use POSITION0",
                ));
            }
            if read(at + 12)? != 3 || bytes[at + 20] != 15 || input > 31 {
                return Err(invalid("Akimbo shader: position input must be float4"));
            }
            found += 1;
        }
    }
    if found != 1 || program.chunk(b"OSGN").is_none() {
        return Err(invalid(
            "Akimbo shader: missing or ambiguous stage signature",
        ));
    }
    let declared = program
        .instructions
        .iter()
        .filter(|instruction| instruction.opcode() == 95)
        .map(Instruction::parsed)
        .collect::<AuthoringResult<Vec<_>>>()?;
    if declared
        .iter()
        .flat_map(|instruction| &instruction.args)
        .filter(|operand| operand.register(1, input) && operand.mask() == Some(15))
        .count()
        != 1
    {
        return Err(invalid(
            "Akimbo shader: POSITION0 needs one float4 input declaration",
        ));
    }
    Ok(())
}

fn layout(program: &Program) -> AuthoringResult<Layout> {
    let (first_body, temps_declaration, temps, cb_declaration, cb_rows) = declarations(program)?;
    let candidates = program
        .instructions
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            decode_candidate(&pair[0], &pair[1]).map(|found| (index, found))
        })
        .collect::<Vec<_>>();
    let [(decode, (input, bone))] = candidates.as_slice() else {
        return Err(invalid(
            "Akimbo shader: expected one rigid vertex bone decoder",
        ));
    };
    let indices = program
        .instructions
        .get(decode + 2..decode + 4)
        .ok_or_else(|| invalid("Akimbo shader: truncated palette index calculation"))?;
    let rows = index_rows(indices, *bone)?;
    validate_signature(program, *input)?;
    Ok(Layout {
        first_body,
        decode: *decode,
        input: *input,
        bone: *bone,
        rows,
        temps_declaration,
        cb_declaration,
        temps,
        cb_rows,
    })
}

/// A dynamic `cb11[relative + 8]` operand, including its component selection.
fn dynamic_palette(operand: &Operand) -> AuthoringResult<Option<Scalar>> {
    if operand.kind() != 8 {
        return Ok(None);
    }
    if operand.indices().is_some() {
        return Ok(None);
    }
    let words = &operand.0;
    if (words[0] >> 20) & 3 == 2
        && (words[0] >> 22) & 63 == 0
        && words.len() == 4
        && words[0] >> 31 != 0
        && [0x41, 0x81, 0xc1].contains(&words[1])
    {
        return Ok(None);
    }
    if words.len() != 5
        || words[0] >> 31 != 0
        || (words[0] >> 20) & 3 != 2
        || (words[0] >> 22) & 7 != 0
        || (words[0] >> 25) & 7 != 3
        || words[1] != 11
        || words[2] != 8
    {
        return Err(invalid(
            "Akimbo shader: unknown dynamic constant-buffer addressing",
        ));
    }
    scalar_source(&Operand(words[3..].to_vec()))
        .map(Some)
        .ok_or_else(|| invalid("Akimbo shader: unsupported palette address register"))
}

fn redirected(operand: &Operand, register: u32) -> Operand {
    // Retain the exact scalar/swizzle selection, substituting only the operand kind and index.
    Operand(vec![(operand.0[0] & 0xfff) | (1 << 20), register])
}

fn writes(instruction: &Instruction, scalar: Scalar) -> bool {
    let destinations = if instruction.opcode() == 38 { 2 } else { 1 };
    instruction.args.iter().take(destinations).any(|operand| {
        operand.register(0, scalar.register)
            && operand
                .mask()
                .is_some_and(|mask| mask & (1 << scalar.lane) != 0)
    })
}

fn temporary_register(operand: &Operand) -> Option<u32> {
    let token = operand.0[0];
    if operand.kind() != 0 || (token >> 20) & 3 != 1 || (token >> 22) & 7 != 0 {
        return None;
    }
    match operand.0.as_slice() {
        [_, register] if token >> 31 == 0 => Some(*register),
        [_, modifier, register] if token >> 31 != 0 && [0x41, 0x81, 0xc1].contains(modifier) => {
            Some(*register)
        }
        _ => None,
    }
}

fn validate_direct_operand(operand: &Operand, temps: u32) -> AuthoringResult<()> {
    if operand.kind() == 8 {
        let start = 1 + usize::from(operand.0[0] >> 31 != 0);
        if let Some(&[11, row]) = operand.0.get(start..)
            && row >= 8
        {
            return Err(invalid(
                "Akimbo shader: direct bone row consumers need a separate adapter",
            ));
        }
    }
    if operand.kind() == 1 && operand.indices().is_none() {
        return Err(invalid(
            "Akimbo shader: modified or indexed vertex inputs need a separate adapter",
        ));
    }
    if operand.kind() == 0 && temporary_register(operand).is_none_or(|register| register >= temps) {
        return Err(invalid(
            "Akimbo shader: temporary address exceeds its declaration",
        ));
    }
    Ok(())
}

fn control_scopes(
    program: &Program,
    allow_control: bool,
) -> AuthoringResult<Vec<Vec<(usize, bool)>>> {
    let mut scope = Vec::new();
    let mut result = Vec::with_capacity(program.instructions.len());
    for (index, instruction) in program.instructions.iter().enumerate() {
        result.push(scope.clone());
        match instruction.opcode() {
            31 if allow_control && scope.len() < 4 && instruction.args.len() == 1 => {
                scope.push((index, false))
            }
            18 if allow_control => {
                let Some((_, in_else)) = scope.last_mut() else {
                    return Err(invalid("Akimbo shader: unmatched else"));
                };
                if *in_else {
                    return Err(invalid("Akimbo shader: repeated else"));
                }
                *in_else = true;
            }
            21 if allow_control && !scope.is_empty() => {
                scope.pop();
            }
            18 | 21 | 31 => return Err(invalid("Akimbo shader: unsupported control flow")),
            _ => {}
        }
    }
    if !scope.is_empty() {
        return Err(invalid("Akimbo shader: unterminated conditional"));
    }
    Ok(result)
}

fn original_body(
    program: &Program,
    layout: &Layout,
    allow_control: bool,
) -> AuthoringResult<Vec<Instruction>> {
    let scopes = control_scopes(program, allow_control)?;
    let mut result = Vec::with_capacity(program.instructions.len());
    let mut live = [true; 3];
    let mut replacements = [0u32; 3];
    for (index, original) in program.instructions.iter().enumerate() {
        if index < layout.first_body {
            result.push(original.clone());
            continue;
        }
        // The established rigid family is straight-line arithmetic plus optional texture load.
        if ![
            0, 14, 16, 17, 26, 28, 35, 38, 45, 50, 51, 52, 54, 56, 62, 64, 68,
        ]
        .contains(&original.opcode())
            && !(allow_control && [18, 21, 31, 49].contains(&original.opcode()))
        {
            return Err(invalid(format!(
                "Akimbo shader: unsupported control flow or vertex operation {} at instruction {index}",
                original.opcode()
            )));
        }
        if original.opcode() == 62 && index + 1 != program.instructions.len() {
            return Err(invalid("Akimbo shader: early return is unsupported"));
        }
        let mut args = original.args.clone();
        for operand in &mut args {
            if let Some(relative) = dynamic_palette(operand)? {
                let axis = layout
                    .rows
                    .iter()
                    .position(|row| *row == relative)
                    .ok_or_else(|| {
                        invalid("Akimbo shader: palette read uses an unknown row index")
                    })?;
                if index < layout.decode + 4
                    || !live[axis]
                    || scopes[index] != scopes[layout.decode]
                {
                    return Err(invalid(
                        "Akimbo shader: palette row read is outside its index lifetime",
                    ));
                }
                *operand = redirected(operand, layout.temps + 1 + axis as u32);
                replacements[axis] += 1;
            } else if operand.register(1, layout.input) {
                if index != layout.decode && operand.lanes().is_none_or(|lanes| lanes.contains(&3))
                {
                    return Err(invalid(
                        "Akimbo shader: position bone lane has an additional consumer",
                    ));
                }
                *operand = redirected(operand, layout.temps + 14);
            } else {
                validate_direct_operand(operand, layout.temps)?;
            }
        }
        if index >= layout.decode + 4 {
            for (axis, row) in layout.rows.iter().enumerate() {
                if writes(original, *row) {
                    live[axis] = false;
                }
            }
        }
        result.push(original.replace(args)?);
    }
    if replacements.contains(&0) {
        return Err(invalid(
            "Akimbo shader: not all affine palette rows are used",
        ));
    }
    Ok(result)
}

fn validate_contract(contract: VertexContract) -> AuthoringResult<()> {
    if !contract.separation.is_finite()
        || !(0.0..=1.0).contains(&contract.separation)
        || contract.separation == 0.0
        || contract.bone_palette == 0
        || contract.bone_palette > 64
        || contract.marker_bone == 0
        || contract.marker_bone >= contract.bone_palette
    {
        return Err(invalid(
            "Akimbo shader: invalid marker, palette or separation contract",
        ));
    }
    Ok(())
}

/// Generate an independent-pose vertex program directly from a structurally compatible donor.
/// Geometry/rig/effect validation and runtime capability checks remain the caller's responsibility.
pub fn transform_vertex(bytes: &[u8], contract: VertexContract) -> AuthoringResult<Vec<u8>> {
    transform_vertex_impl(bytes, contract, contract.bone_palette, true)
}

/// The caller proves the actual source model upload and every indexed source geometry lane
/// against `source_palette`, then remaps into the independently checked final rig. Native
/// shaders can declare fewer rows than their bound buffer contains; that declaration is not
/// the upload-size proof. The generated declaration always covers the complete final palette.
pub fn transform_vertex_with_source_palette(
    bytes: &[u8],
    contract: VertexContract,
    source_palette: u16,
) -> AuthoringResult<Vec<u8>> {
    transform_vertex_impl(bytes, contract, source_palette, false)
}

/// Transform the native packed two-influence declaration. The caller validates both
/// byte bone indices, normalized integer weights, and the source/private palette mapping.
pub fn transform_two_bone_vertex_with_source_palette(
    bytes: &[u8],
    contract: VertexContract,
    source_palette: u16,
) -> AuthoringResult<Vec<u8>> {
    two_bone::transform(bytes, contract, source_palette, None)
}

/// Preserve unmatched weighted object effects and dispatch only for exact generated selectors.
pub fn transform_two_bone_effect_vertex(
    bytes: &[u8],
    bindings: &[EffectBinding],
) -> AuthoringResult<Vec<u8>> {
    effects::validate_bindings(bindings)?;
    let mut bindings = bindings.to_vec();
    bindings.sort_by_key(|binding| binding.quantization_bits);
    two_bone::transform(bytes, bindings[0].contract, 1, Some(&bindings))
}

fn transform_vertex_impl(
    bytes: &[u8],
    contract: VertexContract,
    source_palette: u16,
    require_declared_capacity: bool,
) -> AuthoringResult<Vec<u8>> {
    validate_contract(contract)?;
    if source_palette == 0 || source_palette > contract.bone_palette {
        return Err(invalid(
            "Akimbo shader: source palette exceeds the generated rig",
        ));
    }
    let mut program = Program::read_vertex(bytes)?;
    let layout = layout(&program)?;
    if require_declared_capacity && layout.cb_rows < 8 + u32::from(source_palette) * 3 {
        return Err(invalid(format!(
            "Akimbo shader: native palette declaration ({} rows) does not cover the rig ({source_palette} source bones)",
            layout.cb_rows
        )));
    }
    // Native visibility branches may surround the complete decoder and its consumers.
    // The shared scope/lifetime validator requires every palette read to remain in
    // that same branch and rejects unbalanced, deeper or unsupported control flow.
    let mut body = original_body(&program, &layout, true)?;
    body[layout.temps_declaration].words[1] = layout.temps + 19;
    body[layout.cb_declaration].words[3] = layout
        .cb_rows
        .max(32)
        .max(8 + u32::from(contract.bone_palette) * 3);
    let mut instructions = Vec::with_capacity(body.len() + 100);
    for (index, instruction) in body.into_iter().enumerate() {
        if index == layout.first_body {
            let mut input = temp(layout.input, 0xe4);
            input.0[0] |= 1 << 12;
            instructions.push(ins(54, vec![dst(layout.temps + 14, 15), input]));
        }
        instructions.push(instruction);
        if index == layout.decode + 1 {
            instructions.extend(single_marker::instructions(
                layout.temps,
                layout.bone,
                contract,
            ));
        }
    }
    program.instructions = instructions;
    let output = program.emit()?;
    // The result must remain a vertex program and must pass the container/operand parser again.
    Program::read_vertex(&output)?;
    Ok(output)
}

#[cfg(test)]
#[path = "shader/tests.rs"]
mod tests;
