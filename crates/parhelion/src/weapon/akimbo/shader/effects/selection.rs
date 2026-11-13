//! A bounded decision tree. Splits only choose a candidate; each leaf proves all four words.
use super::*;

const MAX_DEPTH: usize = 48;
// Small straight-line leaves avoid thousands of control-flow joins in the D3D JIT.
// Their fixed bound preserves logarithmic dispatch work and full four-word equality.
const LEAF_PINS: usize = 8;

#[derive(Clone, Copy)]
pub(in super::super) struct Candidate {
    pin: [u32; 4],
    contract: u32,
}

pub(in super::super) enum Selection {
    Leaf(Vec<Candidate>),
    Split {
        axis: u32,
        threshold: u32,
        left: Box<Self>,
        right: Box<Self>,
    },
}

fn build(candidates: Vec<Candidate>, depth: usize) -> AuthoringResult<Selection> {
    if !candidates.is_empty() && candidates.len() <= LEAF_PINS {
        return Ok(Selection::Leaf(candidates));
    }
    if candidates.is_empty() || depth >= MAX_DEPTH {
        return Err(invalid(
            "Akimbo effect selector tree exceeds its bounded depth",
        ));
    }
    let mut best = None;
    for axis in 0..4 {
        let mut sorted = candidates.clone();
        sorted.sort_by_key(|candidate| (candidate.pin[axis], candidate.pin));
        for split in 1..sorted.len() {
            if sorted[split - 1].pin[axis] == sorted[split].pin[axis] {
                continue;
            }
            let score = (
                split.max(sorted.len() - split),
                axis,
                sorted[split].pin[axis],
            );
            if best.as_ref().is_none_or(|(previous, _)| score < *previous) {
                best = Some((score, split));
            }
        }
    }
    let Some(((_, axis, threshold), split)) = best else {
        return Err(invalid("Akimbo effect selector tree has repeated pins"));
    };
    let mut sorted = candidates;
    sorted.sort_by_key(|candidate| (candidate.pin[axis], candidate.pin));
    let right = sorted.split_off(split);
    Ok(Selection::Split {
        axis: axis as u32,
        threshold,
        left: Box::new(build(sorted, depth + 1)?),
        right: Box::new(build(right, depth + 1)?),
    })
}

pub(in super::super) fn new(groups: &[Vec<EffectBinding>]) -> AuthoringResult<Selection> {
    build(
        groups
            .iter()
            .enumerate()
            .flat_map(|(index, group)| {
                group.iter().map(move |binding| Candidate {
                    pin: binding.quantization_bits,
                    contract: index as u32 + 1,
                })
            })
            .collect(),
        0,
    )
}

fn integer(value: u32) -> Operand {
    Operand(vec![1 | (4 << 12), value])
}

impl Selection {
    pub(in super::super) fn instructions(&self, base: u32, output: &mut Vec<Instruction>) {
        let scratch = base + 17;
        match self {
            Self::Leaf(candidates) => {
                output.push(ins(54, vec![dst(base + 19, 1), integer(0)]));
                for candidate in candidates {
                    output.extend([
                        ins(
                            32,
                            vec![
                                dst(scratch, 15),
                                cb(11, 5, 0xe4),
                                literal_vector(candidate.pin),
                            ],
                        ),
                        ins(
                            1,
                            vec![dst(scratch, 3), temp(scratch, 0x04), temp(scratch, 0xae)],
                        ),
                        ins(
                            1,
                            vec![dst(scratch, 1), temp(scratch, 0), temp(scratch, 0x55)],
                        ),
                        ins(
                            55,
                            vec![
                                dst(base + 19, 1),
                                temp(scratch, 0),
                                integer(candidate.contract),
                                temp(base + 19, 0),
                            ],
                        ),
                    ]);
                }
            }
            Self::Split {
                axis,
                threshold,
                left,
                right,
            } => {
                output.push(ins(
                    79,
                    vec![dst(scratch, 1), cb(11, 5, axis * 0x55), integer(*threshold)],
                ));
                output.push(ins(31 | (1 << 18), vec![temp(scratch, 0)]));
                left.instructions(base, output);
                output.push(ins(18, vec![]));
                right.instructions(base, output);
                output.push(ins(21, vec![]));
            }
        }
    }

    #[cfg(test)]
    pub(in super::super) fn depth(&self) -> usize {
        match self {
            Self::Leaf(_) => 0,
            Self::Split { left, right, .. } => 1 + left.depth().max(right.depth()),
        }
    }
}

pub(in super::super) fn transforms(
    base: u32,
    bone: Scalar,
    selector: Scalar,
    groups: &[Vec<EffectBinding>],
    first: usize,
    output: &mut Vec<Instruction>,
) {
    if let [group] = groups {
        output.extend(single_marker::instructions(base, bone, group[0].contract));
        return;
    }
    let middle = groups.len() / 2;
    output.push(ins(
        79,
        vec![
            dst(base + 17, 1),
            selector.source(),
            integer((first + middle + 1) as u32),
        ],
    ));
    output.push(ins(31 | (1 << 18), vec![temp(base + 17, 0)]));
    transforms(base, bone, selector, &groups[..middle], first, output);
    output.push(ins(18, vec![]));
    transforms(
        base,
        bone,
        selector,
        &groups[middle..],
        first + middle,
        output,
    );
    output.push(ins(21, vec![]));
}
