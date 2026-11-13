//! Prove the source GPU declaration before interpreting packed two-influence vertex bytes.
use super::*;
use crate::tag_payload::{read_i64, read_u16, read_u64};

fn table(
    bytes: &[u8],
    at: usize,
    class: u32,
    stride: usize,
    limit: usize,
) -> AuthoringResult<(usize, usize)> {
    if read_u64(bytes, at)? == 0 && read_i64(bytes, at + 8)? == 0 {
        return Ok((0, at + 16));
    }
    let (count, _, rows, actual) = array(bytes, at)?;
    if actual != class
        || count > limit
        || rows
            .checked_add(count * stride)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(invalid(
            "Akimbo GPU declaration has an unsupported typed array",
        ));
    }
    Ok((count, rows))
}

fn roots(manager: &PackageManager) -> AuthoringResult<Vec<(u32, u32, u32)>> {
    let mut result = Vec::new();
    for (tag, _) in manager.get_all_by_reference(0x808072A6) {
        let bytes = read(manager, tag.0)?;
        let elements = read_u32(&bytes, 0xC)?;
        let mapping = read_u32(&bytes, 0x28)?;
        for (child, class) in [(elements, 0x808072AD), (mapping, 0x808072A9)] {
            if manager
                .get_entry(TagHash(child))
                .is_none_or(|entry| entry.reference != class)
            {
                return Err(invalid(
                    "Akimbo render-global declaration reference has the wrong type",
                ));
            }
        }
        result.push((tag.0, elements, mapping));
    }
    if result.is_empty() {
        return Err(invalid("Akimbo source has no native GPU declarations"));
    }
    Ok(result)
}

pub(super) fn sources(manager: &PackageManager) -> AuthoringResult<Vec<u32>> {
    Ok(roots(manager)?
        .into_iter()
        .flat_map(|(root, elements, mapping)| [root, elements, mapping])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn declaration(elements: &[u8], mapping: &[u8], id: u16) -> AuthoringResult<Option<[Vec<u8>; 4]>> {
    let (sets, set_rows) = table(elements, 8, 0x808072AF, 16, 256)?;
    let (count, rows) = table(mapping, 8, 0x808072AC, 28, 256)?;
    let mut found = None;
    for at in (0..count).map(|index| rows + index * 28) {
        if read_u16(mapping, at)? != id {
            continue;
        }
        if mapping[at + 2..at + 8]
            .iter()
            .chain(&mapping[at + 24..at + 28])
            .any(|&byte| byte != 0)
        {
            return Err(invalid(
                "Akimbo weighted layout has unsupported declaration flags or instancing",
            ));
        }
        if found.is_some() {
            return Err(invalid("Akimbo GPU layout identifier is repeated"));
        }
        let mut streams: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::new());
        for (stream, output) in streams.iter_mut().enumerate() {
            let set = read_u32(mapping, at + 8 + stream * 4)?;
            if set == u32::MAX {
                continue;
            }
            if set as usize >= sets {
                return Err(invalid("Akimbo GPU layout names a missing element set"));
            }
            let (count, rows) = table(elements, set_rows + set as usize * 16, 0x808072B2, 3, 32)?;
            output.extend_from_slice(&elements[rows..rows + count * 3]);
        }
        found = Some(streams);
    }
    Ok(found)
}

fn packed(streams: &[Vec<u8>; 4], attribute_stride: u16) -> AuthoringResult<()> {
    // The source format table defines11 as signed normalized int16x4 and6 as uint8x4.
    // The latter packs [bone0,bone1,weight0,weight1] in the native two-influence shaders.
    if streams[0] != [0, 0, 11, 2, 0, 6]
        || streams[1]
            .chunks_exact(3)
            .any(|element| [0, 1, 2].contains(&element[0]))
        || !streams[2].is_empty()
        || !streams[3].is_empty()
    {
        return Err(invalid(
            "Akimbo needs POSITION0 int16x4 and packed BLENDINDICES0 uint8x4 in its first stream",
        ));
    }
    let mut bytes = 0u16;
    let mut semantics = BTreeSet::new();
    for element in streams[1].chunks_exact(3) {
        bytes += match element[2] {
            4 => 16,
            5 | 6 | 10 | 12 => 4,
            11 => 8,
            _ => return Err(invalid("Akimbo weighted attribute format is unknown")),
        };
        if !semantics.insert((element[0], element[1])) {
            return Err(invalid("Akimbo weighted attributes repeat a semantic"));
        }
    }
    if bytes != attribute_stride {
        return Err(invalid(
            "Akimbo GPU attribute declaration differs from the stored stride",
        ));
    }
    Ok(())
}

pub(super) fn validate(
    manager: &PackageManager,
    model: &[u8],
    mesh: usize,
    draws: usize,
    attribute_stride: u16,
) -> AuthoringResult<()> {
    let layouts = (0..draws)
        .map(|draw| stage_layout(model, mesh, draw))
        .collect::<AuthoringResult<BTreeSet<_>>>()?;
    let sources = roots(manager)?
        .into_iter()
        .map(|(_, elements, mapping)| Ok((read(manager, elements)?, read(manager, mapping)?)))
        .collect::<AuthoringResult<Vec<_>>>()?;
    for id in layouts {
        let mut found = None;
        for (elements, mapping) in &sources {
            if let Some(streams) = declaration(elements, mapping, id)? {
                if found.as_ref().is_some_and(|previous| previous != &streams) {
                    return Err(invalid(
                        "Akimbo GPU declarations disagree for the selected layout",
                    ));
                }
                packed(&streams, attribute_stride)?;
                found = Some(streams);
            }
        }
        if found.is_none() {
            return Err(invalid(
                "Weighted draw references an unknown native GPU layout",
            ));
        }
    }
    Ok(())
}

pub(super) fn stage_layout(model: &[u8], mesh: usize, draw: usize) -> AuthoringResult<u16> {
    let ranges = (0..24)
        .map(|stage| read_u16(model, mesh + 0x28 + stage * 2).map(|v| v as i16))
        .collect::<AuthoringResult<Vec<_>>>()?;
    let stages = (0..23)
        .filter(|&stage| {
            let (start, end) = (ranges[stage], ranges[stage + 1]);
            start >= 0 && end >= start && draw >= start as usize && draw < end as usize
        })
        .collect::<Vec<_>>();
    let [stage] = stages.as_slice() else {
        return Err(invalid(
            "Akimbo draw has no unique native stage declaration",
        ));
    };
    read_u16(model, mesh + 0x58 + stage * 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::append_native_array;

    #[test]
    fn akimbo_weighted_layout_refuses_untyped_sets_missing_streams_and_instancing() {
        let mut elements = vec![0; 24];
        append_native_array(&mut elements, 8, 0x808072AF, 1, &[0; 16]).unwrap();
        let (_, sets) = table(&elements, 8, 0x808072AF, 16, 256).unwrap();
        append_native_array(&mut elements, sets, 0x808072B2, 2, &[0, 0, 11, 2, 0, 6]).unwrap();
        let mut mapping = vec![0; 24];
        let mut row = vec![0; 28];
        row[..2].copy_from_slice(&138u16.to_le_bytes());
        row[12..24].fill(255);
        append_native_array(&mut mapping, 8, 0x808072AC, 1, &row).unwrap();
        let (_, rows) = table(&mapping, 8, 0x808072AC, 28, 256).unwrap();
        let streams = declaration(&elements, &mapping, 138).unwrap().unwrap();
        assert_eq!(streams[0], [0, 0, 11, 2, 0, 6]);
        assert!(declaration(&elements, &mapping, 137).unwrap().is_none());
        mapping[rows + 24] = 1;
        assert!(declaration(&elements, &mapping, 138).is_err());
        mapping[rows + 24] = 0;
        mapping[rows + 8..rows + 12].copy_from_slice(&1u32.to_le_bytes());
        assert!(declaration(&elements, &mapping, 138).is_err());
        mapping[rows + 8..rows + 12].fill(0);
        let (_, header, _, _) = array(&elements, sets).unwrap();
        elements[header + 8..header + 12].copy_from_slice(&0x808072AFu32.to_le_bytes());
        assert!(declaration(&elements, &mapping, 138).is_err());
    }

    #[test]
    fn akimbo_two_bone_declaration_proves_byte_placement_and_excludes_other_weight_formats() {
        let streams = [
            vec![0, 0, 11, 2, 0, 6],
            vec![3, 0, 11, 6, 0, 11, 5, 0, 10],
            vec![],
            vec![],
        ];
        packed(&streams, 20).unwrap();
        assert!(packed(&streams, 24).is_err());
        for changed in [
            [vec![0, 0, 11, 2, 0, 5], streams[1].clone(), vec![], vec![]],
            [vec![2, 0, 6, 0, 0, 11], streams[1].clone(), vec![], vec![]],
            [streams[0].clone(), vec![1, 0, 5], vec![], vec![]],
            [
                streams[0].clone(),
                streams[1].clone(),
                vec![2, 0, 6],
                vec![],
            ],
        ] {
            assert!(packed(&changed, 20).is_err());
        }
    }
}
