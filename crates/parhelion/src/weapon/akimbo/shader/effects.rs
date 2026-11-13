//! The shared object-effect path keeps native rows unless a complete generated model pin matches.
use super::*;

#[path = "effects/selection.rs"]
pub(super) mod selection;

#[derive(Clone, Copy, Debug)]
pub struct EffectBinding {
    /// Exact descriptor bytes +0x60..0x70, supplied to the vertex program as cb11[5].
    /// The caller must establish that these bits occur only on its generated descriptors.
    pub quantization_bits: [u32; 4],
    pub contract: VertexContract,
}

const MAX_BINDINGS: usize = 4096;
const MAX_PROGRAM_BYTES: usize = 2 * 1024 * 1024;

fn literal_vector(values: [u32; 4]) -> Operand {
    let mut words = vec![2 | (4 << 12)];
    words.extend(values);
    Operand(words)
}

pub(super) fn validate_bindings(bindings: &[EffectBinding]) -> AuthoringResult<()> {
    if bindings.is_empty() || bindings.len() > MAX_BINDINGS {
        return Err(invalid(
            "Akimbo effect shader: expected 1..4096 generated model bindings",
        ));
    }
    let mut occupied = std::collections::BTreeSet::new();
    for binding in bindings {
        validate_contract(binding.contract)?;
        if binding
            .quantization_bits
            .iter()
            .any(|bits| !f32::from_bits(*bits).is_finite())
            || f32::from_bits(binding.quantization_bits[3]) <= 0.0
            || !occupied.insert(binding.quantization_bits)
        {
            return Err(invalid(
                "Akimbo effect shader: invalid or repeated model quantization pin",
            ));
        }
    }
    Ok(())
}

/// Several model parts can share one transform, but each retains its complete exact pin.
pub(super) fn groups(bindings: &[EffectBinding]) -> Vec<Vec<EffectBinding>> {
    let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
    for binding in bindings {
        let contract = binding.contract;
        groups
            .entry((
                contract.marker_bone,
                contract.bone_palette,
                contract.separation.to_bits(),
            ))
            .or_default()
            .push(*binding);
    }
    groups
        .into_values()
        .map(|mut group| {
            group.sort_by_key(|binding| binding.quantization_bits);
            group
        })
        .collect()
}

pub(super) fn emit(program: Program) -> AuthoringResult<Vec<u8>> {
    if program
        .instructions
        .iter()
        .map(|instruction| instruction.words.len())
        .sum::<usize>()
        > MAX_PROGRAM_BYTES / 4
    {
        return Err(invalid(
            "Akimbo effect shader exceeds its 2 MiB program budget",
        ));
    }
    let output = program.emit()?;
    if output.len() > MAX_PROGRAM_BYTES {
        return Err(invalid(
            "Akimbo effect shader exceeds its 2 MiB container budget",
        ));
    }
    Program::read_vertex(&output)?;
    Ok(output)
}

pub(super) fn original_rows(base: u32, bone: Scalar) -> Vec<Instruction> {
    let mut output = vec![
        ins(54, vec![dst(base + 18, 1), bone.source()]),
        ins(
            38,
            vec![
                Operand(vec![13 << 12]),
                dst(base, 2),
                bone.source(),
                Operand(vec![1 | (4 << 12), 3]),
            ],
        ),
    ];
    for axis in 0..3 {
        let mut words = vec![
            2 | (1 << 2) | (0xe4 << 4) | (8 << 12) | (2 << 20) | (3 << 25),
            11,
            8 + axis,
        ];
        words.extend(
            Scalar {
                register: base,
                lane: 1,
            }
            .source()
            .0,
        );
        output.push(ins(54, vec![dst(base + 1 + axis, 15), Operand(words)]));
    }
    output
}

pub(super) fn generated_rows(
    base: u32,
    bone: Scalar,
    selector: Scalar,
    groups: &[Vec<EffectBinding>],
) -> Vec<Instruction> {
    // No private marker or flag interpretation is permitted before an exact full pin match.
    let mut output = vec![ins(31 | (1 << 18), vec![selector.source()])];
    selection::transforms(base, bone, selector, groups, 0, &mut output);
    output.push(ins(21, vec![]));
    output.push(ins(54, vec![bone.destination(), temp(base + 18, 0)]));
    output
}

/// Generate the shared rigid object-effect vertex path for a complete installed generation.
///
/// This function does not infer that a model pin is unique. Before installing shared shader
/// overrides the compiler must scan every source descriptor, reject any native collision and
/// include all generated pins in the generation's package fingerprint. Unmatched models keep
/// their full native bone index, even when its high bits resemble the private copy flags.
/// Each generated descriptor's validated rig supplies its actual upload size. The original
/// shader declaration describes neither that new upload nor all unrelated native uploads.
pub fn transform_effect_vertex(
    bytes: &[u8],
    bindings: &[EffectBinding],
) -> AuthoringResult<Vec<u8>> {
    validate_bindings(bindings)?;
    let groups = groups(bindings);
    let selection = selection::new(&groups)?;
    let mut program = Program::read_vertex(bytes)?;
    let layout = layout(&program)?;
    if layout.temps > 4076 {
        return Err(invalid(
            "Akimbo effect shader: insufficient temporary capacity",
        ));
    }
    let mut rows = layout.cb_rows.max(32);
    for binding in bindings {
        rows = rows.max(8 + u32::from(binding.contract.bone_palette) * 3);
    }
    let mut body = original_body(&program, &layout, true)?;
    body[layout.temps_declaration].words[1] = layout.temps + 20;
    body[layout.cb_declaration].words[3] = rows;
    let mut instructions = Vec::with_capacity(body.len() + bindings.len() * 4 + groups.len() * 100);
    for (index, instruction) in body.into_iter().enumerate() {
        if index == layout.first_body {
            let mut input = temp(layout.input, 0xe4);
            input.0[0] |= 1 << 12;
            instructions.push(ins(54, vec![dst(layout.temps + 14, 15), input]));
        }
        instructions.push(instruction);
        if index == layout.decode + 1 {
            instructions.extend(original_rows(layout.temps, layout.bone));
            selection.instructions(layout.temps, &mut instructions);
            instructions.extend(generated_rows(
                layout.temps,
                layout.bone,
                Scalar {
                    register: layout.temps + 19,
                    lane: 0,
                },
                &groups,
            ));
        }
    }
    program.instructions = instructions;
    emit(program)
}
