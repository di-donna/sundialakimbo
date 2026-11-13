//! Source-derived paired buffers and materials. Allocation remains owned by `linking`.
use super::*;
use crate::tag_payload::{read_u16, read_u64, write_u64};
use crate::weapon::akimbo::{geometry, shader};

struct Buffer {
    tag: u32,
    data_tag: u32,
    header: Vec<u8>,
    data: Vec<u8>,
}

fn buffer(manager: &PackageManager, tag: u32, subtype: u8) -> AuthoringResult<Buffer> {
    let entry = manager
        .get_entry(TagHash(tag))
        .filter(|e| e.file_type == 32 && e.file_subtype == subtype)
        .ok_or_else(|| invalid("Akimbo requires native vertex and 16-bit index buffers"))?;
    let data_entry = manager
        .get_entry(TagHash(entry.reference))
        .ok_or_else(|| invalid("Akimbo buffer data is absent"))?;
    if data_entry.file_size > 32 * 1024 * 1024 {
        return Err(invalid(
            "Akimbo buffer exceeds the checked allocation limit",
        ));
    }
    let header = read(manager, tag)?;
    let data = read(manager, entry.reference)?;
    let length = if subtype == 6 {
        if header.get(1) != Some(&0) || !data.len().is_multiple_of(2) {
            return Err(invalid("Akimbo currently requires 16-bit indices"));
        }
        read_u64(&header, 8)?
    } else {
        u64::from(read_u32(&header, 0)?)
    };
    if length != data.len() as u64 {
        return Err(invalid(
            "Akimbo buffer header length differs from its payload",
        ));
    }
    Ok(Buffer {
        tag,
        data_tag: entry.reference,
        header,
        data,
    })
}

struct Mesh {
    duplicate: bool,
    at: usize,
    record_rows: usize,
    positions: Buffer,
    attributes: Buffer,
    indices: Buffer,
    index_values: Vec<u16>,
    draws: Vec<geometry::Draw>,
}

/// A material's native palette declaration need only cover the vertices drawn with that
/// material. The model-wide palette can also include moving bones used by other materials.
fn draw_palette(
    positions: &[u8],
    indices: &[u16],
    draw: &geometry::Draw,
    stride: u16,
) -> AuthoringResult<u16> {
    if ![8, 12].contains(&stride) {
        return Err(invalid("Unsupported akimbo source vertex stride"));
    }
    let start = draw.first_index as usize;
    let end = start
        .checked_add(draw.index_count as usize)
        .ok_or_else(|| invalid("Akimbo draw index range overflow"))?;
    let range = indices
        .get(start..end)
        .ok_or_else(|| invalid("Akimbo draw index range exceeds its source buffer"))?;
    let mut palette = 0;
    for &index in range.iter().filter(|&&index| index != u16::MAX) {
        let vertex = usize::from(index)
            .checked_mul(usize::from(stride))
            .and_then(|offset| positions.get(offset..offset + usize::from(stride)))
            .ok_or_else(|| invalid("Akimbo draw vertex exceeds its source buffer"))?;
        let required = (if stride == 12 {
            u16::from(vertex[8].max(vertex[9]))
        } else {
            u16::from_le_bytes([vertex[6], vertex[7]])
        })
        .checked_add(1)
        .ok_or_else(|| invalid("Akimbo draw source palette overflow"))?;
        palette = palette.max(required);
    }
    if palette == 0 {
        return Err(invalid("Akimbo paired draw has no source vertices"));
    }
    Ok(palette)
}

fn materialless_pairing(
    draws: &mut [geometry::Draw],
    present: &[bool],
    layouts: &[u16],
    stage_zero: std::ops::Range<usize>,
) -> AuthoringResult<()> {
    if present.len() != draws.len() || layouts.len() != draws.len() {
        return Err(invalid(
            "Akimbo draw classification has inconsistent lengths",
        ));
    }
    for index in 0..draws.len() {
        if present[index] {
            continue;
        }
        if !stage_zero.contains(&index) {
            return Err(invalid(
                "Akimbo absent materials are only supported in the native stage-zero range",
            ));
        }
        let draw = draws[index];
        let candidates = draws
            .iter()
            .enumerate()
            .filter(|(other, candidate)| {
                present[*other]
                    && candidate.first_index == draw.first_index
                    && candidate.index_count == draw.index_count
                    && candidate.primitive_count == draw.primitive_count
                    && candidate.topology == draw.topology
                    && layouts[*other] == layouts[index]
            })
            .map(|(_, draw)| draw.duplicate)
            .collect::<BTreeSet<_>>();
        let [true] = candidates.into_iter().collect::<Vec<_>>()[..] else {
            return Err(invalid(
                "Akimbo material-less draw requires an identical, unambiguous validated material range and vertex layout",
            ));
        };
        draws[index].duplicate = true;
    }
    Ok(())
}

fn meshes(manager: &PackageManager, model: &[u8]) -> AuthoringResult<Vec<Mesh>> {
    let (count, _, rows, class) = array(model, 0x10)?;
    if class != MESH_ROW_CLASS || count == 0 || count > 256 {
        return Err(invalid("Akimbo model has no supported mesh table"));
    }
    (0..count)
        .map(|index| {
            let at = rows + index * 136;
            if [8, 12]
                .into_iter()
                .any(|offset| read_u32(model, at + offset).ok() != Some(u32::MAX))
            {
                return Err(invalid(
                    "Akimbo does not yet support auxiliary or weighted vertex streams",
                ));
            }
            let positions = buffer(manager, read_u32(model, at)?, 4)?;
            let attributes = buffer(manager, read_u32(model, at + 4)?, 4)?;
            let indices = buffer(manager, read_u32(model, at + 16)?, 6)?;
            let index_values = indices
                .data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let (records, _, record_rows, class) = array(model, at + 24)?;
            if class != 0x8080737e || records == 0 || records > 65536 {
                return Err(invalid(
                    "Akimbo model draw records have an unsupported layout",
                ));
            }
            let draws = (0..records)
                .map(|n| {
                    let row = record_rows + n * 32;
                    if read_u16(model, row + 4)? != u16::MAX {
                        return Err(invalid(
                            "Akimbo does not yet support external material selectors",
                        ));
                    }
                    Ok(geometry::Draw {
                        first_index: read_u32(model, row + 8)?,
                        index_count: read_u32(model, row + 12)?,
                        primitive_count: read_u32(model, row + 16)?,
                        topology: match read_u16(model, row + 6)? {
                            4 => geometry::Topology::TriangleList,
                            5 => geometry::Topology::TriangleStrip,
                            _ => return Err(invalid("Akimbo requires triangle lists or strips")),
                        },
                        duplicate: true,
                    })
                })
                .collect::<AuthoringResult<Vec<_>>>()?;
            let mut draws = draws;
            let mut present = Vec::with_capacity(draws.len());
            for (draw_index, draw) in draws.iter_mut().enumerate() {
                let material_tag = read_u32(model, record_rows + draw_index * 32)?;
                present.push(material_tag != u32::MAX);
                if material_tag == u32::MAX {
                    continue;
                }
                let material = read(manager, material_tag).map_err(|error| {
                    error.context(format!(
                        "Mesh {index}, draw {draw_index}, material 0x{material_tag:08X}"
                    ))
                })?;
                let vertex = read_u32(&material, 0x48)?;
                let entry = manager
                    .get_entry(TagHash(vertex))
                    .filter(|e| e.file_type == 33 && e.file_subtype == 1)
                    .ok_or_else(|| invalid("Akimbo draw has no native vertex program"))?;
                draw.duplicate = !shader::retained_reticle(&read(manager, entry.reference)?)?;
            }
            if present.contains(&false) {
                if read_u16(&positions.header, 4)? != 8 {
                    return Err(invalid(
                        "Akimbo absent-material ranges require validated rigid companion draws",
                    ));
                }
                let layouts = (0..draws.len())
                    .map(|draw| super::akimbo_layout::stage_layout(model, at, draw))
                    .collect::<AuthoringResult<Vec<_>>>()?;
                let start = read_u16(model, at + 0x28)? as i16;
                let end = read_u16(model, at + 0x2A)? as i16;
                if start < 0 || end < start {
                    return Err(invalid("Akimbo stage-zero range is invalid"));
                }
                materialless_pairing(&mut draws, &present, &layouts, start as usize..end as usize)?;
            }
            let duplicate = draws.iter().any(|draw| draw.duplicate);
            Ok(Mesh {
                duplicate,
                at,
                record_rows,
                positions,
                attributes,
                indices,
                index_values,
                draws,
            })
        })
        .collect()
}

fn emit_buffer(
    prefix: &str,
    source: &Buffer,
    data: Vec<u8>,
    index: bool,
    nodes: &mut Vec<Node>,
) -> AuthoringResult<String> {
    let data_symbol = format!("{prefix}-data");
    let header_symbol = format!("{prefix}-header");
    let mut header = Node::new(&header_symbol, source.tag, source.header.clone());
    if index {
        write_u64(&mut header.payload, 8, data.len() as u64)?;
    } else {
        write_u32(
            &mut header.payload,
            0,
            u32::try_from(data.len())
                .map_err(|_| invalid("Akimbo buffer exceeds native length"))?,
        )?;
    }
    header.reference = Some(data_symbol.clone());
    nodes.push(Node::new(data_symbol, source.data_tag, data));
    nodes.push(header);
    Ok(header_symbol)
}

pub(super) fn model(
    manager: &PackageManager,
    prefix: &str,
    model: &mut Node,
    nodes: &mut Vec<Node>,
    glow: bool,
    gap: f32,
    pose: Option<&crate::weapon::akimbo::visual::pose::Prepared>,
) -> AuthoringResult<bool> {
    let native_palette = u16::try_from(read_u32(&model.payload, BONE_PALETTE)?)
        .map_err(|_| invalid("Akimbo bone palette is too large"))?;
    let mut meshes = meshes(manager, &model.payload)
        .map_err(|error| error.context(format!("Akimbo source model {}", model.template)))?;
    if meshes.iter().all(|mesh| !mesh.duplicate) {
        return Ok(false);
    }
    let source_count = pose.map_or(5, |pose| pose.renumbering.map.len() - 1);
    let marker = pose.map_or(3, |pose| pose.pose.marker_bone as u16);
    if native_palette == 0 || usize::from(native_palette) > source_count {
        return Err(invalid(
            "Akimbo model palette exceeds its validated source weapon rig",
        ));
    }
    // Static parts often upload only bone zero. Keep validating their original lanes against
    // that native palette, then extend the private upload to include the audited marker bone.
    let mut material_palettes = BTreeMap::<u32, (u16, bool)>::new();
    for mesh in &mut meshes {
        let stride = read_u16(&mesh.positions.header, 4)?;
        let weighted = stride == 12;
        if ![8, 12].contains(&stride)
            || !mesh
                .positions
                .data
                .len()
                .is_multiple_of(usize::from(stride))
        {
            return Err(invalid(format!(
                "Akimbo requires rigid or packed two-influence position vertices: buffer 0x{:08X}, stride {}, {} payload bytes",
                mesh.positions.tag,
                read_u16(&mesh.positions.header, 4)?,
                mesh.positions.data.len(),
            )));
        }
        if weighted {
            super::akimbo_layout::validate(
                manager,
                &model.payload,
                mesh.at,
                mesh.draws.len(),
                read_u16(&mesh.attributes.header, 4)?,
            )?;
        }
        for vertex in mesh.positions.data.chunks_exact(usize::from(stride)) {
            let bone = u16::from_le_bytes([vertex[6], vertex[7]]);
            if weighted && (bone != 32767 || u16::from(vertex[10]) + u16::from(vertex[11]) != 255) {
                return Err(invalid(
                    "Packed two-influence source has an unsupported selector or weight sum",
                ));
            }
            let bones = if weighted {
                [u16::from(vertex[8]), u16::from(vertex[9])]
            } else {
                [bone, bone]
            };
            if bones.into_iter().any(|bone| bone >= native_palette) {
                return Err(invalid(
                    "Akimbo source vertex exceeds its native bone palette",
                ));
            }
        }
        for (index, draw) in mesh
            .draws
            .iter()
            .enumerate()
            .filter(|(_, draw)| draw.duplicate)
        {
            let material = read_u32(&model.payload, mesh.record_rows + index * 32)?;
            if material == u32::MAX {
                continue;
            }
            let required = draw_palette(&mesh.positions.data, &mesh.index_values, draw, stride)?;
            if let Some((palette, previous)) = material_palettes.get_mut(&material) {
                if *previous != weighted {
                    return Err(invalid(
                        "One akimbo material names incompatible vertex formats",
                    ));
                }
                *palette = (*palette).max(required);
            } else {
                material_palettes.insert(material, (required, weighted));
            }
        }
        for vertex in mesh.positions.data.chunks_exact_mut(usize::from(stride)) {
            if weighted {
                vertex[6..8].fill(0);
                if let Some(pose) = pose {
                    for at in [8, 9] {
                        vertex[at] = u8::try_from(pose.renumbering.map[usize::from(vertex[at])])
                            .map_err(|_| invalid("Weighted private bone exceeds byte index"))?;
                    }
                }
            } else if let Some(pose) = pose {
                let bone = u16::from_le_bytes([vertex[6], vertex[7]]);
                vertex[6..8].copy_from_slice(
                    &(pose.renumbering.map[usize::from(bone)] as u16).to_le_bytes(),
                );
            }
        }
    }
    let palette = pose.map_or(native_palette.max(marker + 1), |pose| {
        pose.pose.weapon_bone_count as u16
    });
    write_u32(&mut model.payload, BONE_PALETTE, u32::from(palette))?;
    let parts = meshes
        .iter()
        .map(|mesh| {
            Ok(geometry::Part {
                positions: &mesh.positions.data,
                position_stride: read_u16(&mesh.positions.header, 4)?,
                attributes: &mesh.attributes.data,
                attribute_stride: read_u16(&mesh.attributes.header, 4)?,
                indices: &mesh.index_values,
                draws: &mesh.draws,
                duplicate: mesh.duplicate,
            })
        })
        .collect::<AuthoringResult<Vec<_>>>()?;
    let mut offset = [0.0; 3];
    for (axis, value) in offset.iter_mut().enumerate() {
        *value = f32::from_bits(read_u32(&model.payload, POSITION_OFFSET + axis * 4)?);
    }
    let prepared = geometry::prepare(
        geometry::Source {
            position_scale: f32::from_bits(read_u32(&model.payload, 0x6c)?),
            position_offset: offset,
            bone_palette: palette,
            parts: &parts,
        },
        geometry::Settings {
            separation: gap,
            marker_bone: marker,
            max_position_error: 0.0001,
        },
    )
    .map_err(|e| invalid(format!("Model {}: {e}", model.template)))?;
    // UV scale/offset at 0x70..0x7C remain native. Position quantization has one scalar.
    write_u32(&mut model.payload, 0x6c, prepared.position_scale.to_bits())?;
    for (axis, value) in prepared.position_offset.into_iter().enumerate() {
        write_u32(
            &mut model.payload,
            0x50 + axis * 4,
            prepared.position_scale.to_bits(),
        )?;
        write_u32(&mut model.payload, 0x80 + axis * 4, value.to_bits())?;
    }
    write_u32(&mut model.payload, 0x8c, prepared.sphere_radius.to_bits())?;
    let mut materials = BTreeMap::<u32, String>::new();
    for (index, (source, prepared)) in meshes.iter().zip(prepared.parts).enumerate() {
        for (slot, buffer, payload, is_index, label) in [
            (0, &source.positions, prepared.positions, false, "positions"),
            (
                4,
                &source.attributes,
                prepared.attributes,
                false,
                "attributes",
            ),
            (
                16,
                &source.indices,
                prepared
                    .indices
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                true,
                "indices",
            ),
        ] {
            let symbol = emit_buffer(
                &format!("{prefix}-akimbo-{index}-{label}"),
                buffer,
                payload,
                is_index,
                nodes,
            )?;
            model.patch(source.at + slot, symbol)?;
        }
        for (record, draw) in prepared.draws.iter().enumerate() {
            let at = source.record_rows + record * 32;
            write_u32(&mut model.payload, at + 8, draw.first_index)?;
            write_u32(&mut model.payload, at + 12, draw.index_count)?;
            write_u32(&mut model.payload, at + 16, draw.primitive_count)?;
            if !source.duplicate || !draw.duplicate {
                continue;
            }
            let tag = read_u32(&model.payload, at)?;
            // Keep the native absent material absent; only its shared geometry range moves.
            if tag == u32::MAX {
                continue;
            }
            let symbol = if let Some(symbol) = materials.get(&tag) {
                symbol.clone()
            } else {
                let symbol = material(
                    manager,
                    &format!("{prefix}-akimbo-{tag:08X}"),
                    tag,
                    palette,
                    nodes,
                    glow,
                    gap,
                    marker,
                    material_palettes[&tag].0,
                    material_palettes[&tag].1,
                )?;
                materials.insert(tag, symbol.clone());
                symbol
            };
            model.patch(at, symbol)?;
        }
    }
    Ok(true)
}

fn material(
    manager: &PackageManager,
    prefix: &str,
    tag: u32,
    palette: u16,
    nodes: &mut Vec<Node>,
    glow: bool,
    gap: f32,
    marker: u16,
    source_palette: u16,
    weighted: bool,
) -> AuthoringResult<String> {
    manager
        .get_entry(TagHash(tag))
        .filter(|e| e.reference == 0x808071e8)
        .ok_or_else(|| invalid("Akimbo material is not a native gear material"))?;
    let material = read(manager, tag)?;
    let vertex = read_u32(&material, 0x48)?;
    let entry = manager
        .get_entry(TagHash(vertex))
        .filter(|e| e.file_type == 33 && e.file_subtype == 1)
        .ok_or_else(|| invalid("Akimbo material has no supported vertex program"))?;
    manager
        .get_entry(TagHash(entry.reference))
        .filter(|e| e.file_type == 41 && e.file_subtype == 1 && e.reference == vertex)
        .ok_or_else(|| invalid("Akimbo vertex header and payload are not reciprocal"))?;
    let header = read(manager, vertex)?;
    let code = read(manager, entry.reference)?;
    if header.len() != 40
        || read_u64(&header, 0)? != 40
        || read_u32(&header, 8)? as usize != code.len()
        || read_u32(&header, 12)? != u32::MAX
        || header[16..].iter().any(|&b| b != 0)
    {
        return Err(invalid("Akimbo vertex allocation header is unsupported"));
    }
    let transform = if weighted {
        shader::transform_two_bone_vertex_with_source_palette
    } else {
        shader::transform_vertex_with_source_palette
    };
    let code = transform(
        &code,
        shader::VertexContract {
            marker_bone: marker,
            bone_palette: palette,
            separation: gap,
        },
        source_palette,
    )
    .map_err(|e| {
        e.context(format!(
            "Akimbo material 0x{tag:08X}, vertex 0x{vertex:08X}"
        ))
    })?;
    let material_symbol = format!("{prefix}-material");
    let vertex_symbol = format!("{prefix}-vertex");
    let code_symbol = format!("{prefix}-code");
    let mut material_node = Node::new(&material_symbol, tag, material);
    material_node.patch(0x48, &vertex_symbol)?;
    if glow {
        glow_pixel(manager, prefix, &mut material_node, nodes)?;
    }
    let mut vertex_node = Node::new(&vertex_symbol, vertex, header);
    write_u32(
        &mut vertex_node.payload,
        8,
        u32::try_from(code.len()).map_err(|_| invalid("Akimbo shader is too large"))?,
    )?;
    vertex_node.reference = Some(code_symbol.clone());
    let mut code_node = Node::new(code_symbol, entry.reference, code);
    code_node.reference = Some(vertex_symbol);
    nodes.extend([material_node, vertex_node, code_node]);
    Ok(material_symbol)
}

fn glow_pixel(
    manager: &PackageManager,
    prefix: &str,
    material: &mut Node,
    nodes: &mut Vec<Node>,
) -> AuthoringResult<()> {
    if read_u32(&material.payload, 24)? & 0x1c00_0000 == 0 {
        return Ok(());
    }
    let pixel = read_u32(&material.payload, 0x2c8)?;
    if [0, u32::MAX].contains(&pixel) {
        return Ok(());
    }
    let entry = manager
        .get_entry(TagHash(pixel))
        .filter(|e| e.file_type == 33 && e.file_subtype == 0)
        .ok_or_else(|| invalid("Akimbo glow material has no native pixel program"))?;
    manager
        .get_entry(TagHash(entry.reference))
        .filter(|e| e.file_type == 41 && e.file_subtype == 0 && e.reference == pixel)
        .ok_or_else(|| invalid("Akimbo glow pixel references are not reciprocal"))?;
    let mut header = read(manager, pixel)?;
    let code = read(manager, entry.reference)?;
    if header.len() != 40
        || read_u64(&header, 0)? != 40
        || read_u32(&header, 8)? as usize != code.len()
        || read_u32(&header, 12)? != u32::MAX
        || header[16..].iter().any(|&b| b != 0)
    {
        return Err(invalid(
            "Akimbo glow pixel allocation header is unsupported",
        ));
    }
    let crate::weapon::glow::Change::Patched(code) = crate::weapon::glow::patch(&code)? else {
        return Ok(());
    };
    let header_symbol = format!("{prefix}-glow-pixel");
    let code_symbol = format!("{prefix}-glow-code");
    write_u32(
        &mut header,
        8,
        u32::try_from(code.len()).map_err(|_| invalid("Akimbo glow program is too large"))?,
    )?;
    let mut header = Node::new(&header_symbol, pixel, header);
    header.reference = Some(code_symbol.clone());
    let mut data = Node::new(&code_symbol, entry.reference, code);
    data.reference = Some(header_symbol.clone());
    material.patch(0x2c8, header_symbol)?;
    nodes.extend([header, data]);
    Ok(())
}

pub(super) fn effect_binding(
    model: &[u8],
    gap: f32,
    marker: u16,
) -> AuthoringResult<shader::EffectBinding> {
    let mut pin = [0; 4];
    for (index, value) in pin.iter_mut().enumerate() {
        *value = read_u32(model, 0x60 + index * 4)?;
    }
    Ok(shader::EffectBinding {
        quantization_bits: pin,
        contract: shader::VertexContract {
            marker_bone: marker,
            bone_palette: u16::try_from(read_u32(model, BONE_PALETTE)?)
                .map_err(|_| invalid("Akimbo palette overflow"))?,
            separation: gap,
        },
    })
}

/// The native rigid and packed two-influence object-effect shaders in this content build.
/// Each replacement dispatches only on a generated model's exact quantization contract.
pub(super) fn effects(
    manager: &PackageManager,
    emission: &PackageEmission,
    generated: &[(TagHash, shader::EffectBinding)],
    replacements: &mut Vec<ReplacementSpec>,
) -> AuthoringResult<()> {
    if generated.is_empty() {
        return Ok(());
    }
    let owned = generated
        .iter()
        .map(|(tag, _)| *tag)
        .collect::<BTreeSet<_>>();
    let mut unique = BTreeMap::<[u32; 4], shader::EffectBinding>::new();
    for (_, binding) in generated {
        if let Some(previous) = unique.insert(binding.quantization_bits, *binding)
            && (previous.contract.marker_bone != binding.contract.marker_bone
                || previous.contract.bone_palette != binding.contract.bone_palette
                || previous.contract.separation.to_bits() != binding.contract.separation.to_bits())
        {
            return Err(invalid(
                "Two generated akimbo models share an effect selector with different pose contracts",
            ));
        }
    }
    let pin = |bytes: &[u8]| -> AuthoringResult<[u32; 4]> {
        let mut result = [0; 4];
        for (index, value) in result.iter_mut().enumerate() {
            *value = read_u32(bytes, 0x60 + index * 4)?;
        }
        Ok(result)
    };
    // A high native bone index must never be mistaken for a private copy flag. Check every
    // descriptor in the source generation, not just weapons or the selected donor's parts.
    for (tag, _) in manager.get_all_by_reference(MODEL_CLASS) {
        if unique.contains_key(&pin(&read(manager, tag.0)?)?) {
            return Err(invalid(format!(
                "Akimbo effect selector collides with source model {tag}; this quantization cannot be published"
            )));
        }
    }
    for package in &emission.asset_packages.packages {
        let allocator = AppendedTagAllocator::new(package.id, 0);
        for (index, node) in package.tags.iter().enumerate() {
            let tag = allocator.assigned_tag(index, "Akimbo effects", "model")?;
            if !owned.contains(&tag)
                && manager
                    .get_entry(node.template_tag)
                    .is_some_and(|e| e.reference == MODEL_CLASS)
                && unique.contains_key(&pin(&node.payload)?)
            {
                return Err(invalid(
                    "Akimbo effect selector collides with another authored model",
                ));
            }
        }
    }
    let bindings = unique.into_values().collect::<Vec<_>>();
    for (vertex, weighted) in [
        (0x80feba62, false),
        (0x80feba67, false),
        (0x80feba6d, false),
        (0x80feba73, false),
        (0x80feba81, false),
        (0x80feba87, false),
        (0x80feba60, true),
        (0x80feba66, true),
        (0x80feba6b, true),
        (0x80feba71, true),
        (0x80feba7f, true),
        (0x80feba86, true),
    ] {
        let entry = manager
            .get_entry(TagHash(vertex))
            .filter(|e| e.file_type == 33 && e.file_subtype == 1)
            .ok_or_else(|| invalid("The supported global akimbo effect shader is absent"))?;
        manager
            .get_entry(TagHash(entry.reference))
            .filter(|e| e.file_type == 41 && e.file_subtype == 1 && e.reference == vertex)
            .ok_or_else(|| invalid("The global effect shader reference is not reciprocal"))?;
        let mut header = read(manager, vertex)?;
        let code = read(manager, entry.reference)?;
        if header.len() != 40
            || read_u64(&header, 0)? != 40
            || read_u32(&header, 8)? as usize != code.len()
            || read_u32(&header, 12)? != u32::MAX
            || header[16..].iter().any(|&b| b != 0)
        {
            return Err(invalid(
                "Global akimbo effect allocation header is unsupported",
            ));
        }
        let code = if weighted {
            shader::transform_two_bone_effect_vertex(&code, &bindings)?
        } else {
            shader::transform_effect_vertex(&code, &bindings)?
        };
        write_u32(
            &mut header,
            8,
            u32::try_from(code.len()).map_err(|_| invalid("Akimbo effect shader is too large"))?,
        )?;
        replacements.push(ReplacementSpec {
            tag: TagHash(vertex),
            payload: header,
        });
        replacements.push(ReplacementSpec {
            tag: TagHash(entry.reference),
            payload: code,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn akimbo_absent_material_preserves_a_proven_shared_range_without_inventing_a_shader() {
        let draw = geometry::Draw {
            first_index: 0,
            index_count: 4,
            primitive_count: 2,
            topology: geometry::Topology::TriangleStrip,
            duplicate: true,
        };
        let mut draws = [draw, draw, draw];
        materialless_pairing(&mut draws, &[false, true, true], &[139; 3], 0..1).unwrap();
        assert!(draws[0].duplicate);
        draws[1].duplicate = false;
        draws[2].duplicate = false;
        assert!(materialless_pairing(&mut draws, &[false, true, true], &[139; 3], 0..1).is_err());
        draws[2].duplicate = true;
        assert!(materialless_pairing(&mut draws, &[false, true, true], &[139; 3], 0..1).is_err());
        assert!(materialless_pairing(&mut draws, &[false, false, false], &[139; 3], 0..1).is_err());
        assert!(
            materialless_pairing(&mut draws, &[false, true, true], &[138, 139, 139], 0..1).is_err()
        );
        draws[0].first_index = 1;
        assert!(materialless_pairing(&mut draws, &[false, true, true], &[139; 3], 0..1).is_err());
        assert!(
            materialless_pairing(&mut [draw; 3], &[true, false, true], &[139; 3], 0..1).is_err()
        );
    }

    #[test]
    #[ignore = "requires clean SUNDIAL_STOCK_PACKAGES; reads stock assets without installing"]
    fn akimbo_stock_absent_material_keeps_native_stage_metadata_and_shared_geometry() {
        let packages = crate::test_support::stock_packages();
        sundial::package_authoring::initialize_source_decoder(&packages).unwrap();
        let manager =
            sundial::package_authoring::open_shadowkeep_package_manager(&packages).unwrap();
        let pose = crate::weapon::akimbo::visual::pose::prepare(
            &read(&manager, 0x80BBBF36).unwrap(),
            &read(&manager, 0x80BC8631).unwrap(),
            |tag| read(&manager, tag),
        )
        .unwrap();
        let original = read(&manager, 0x81528E68).unwrap();
        let mut paired = Node::new("absent-material-model", 0x81528E68, original.clone());
        let mut nodes = Vec::new();
        assert!(
            model(
                &manager,
                "absent-material",
                &mut paired,
                &mut nodes,
                false,
                0.36,
                Some(&pose)
            )
            .unwrap()
        );
        let (count, _, meshes, _) = array(&original, 0x10).unwrap();
        assert_eq!(count, 1);
        let (count, _, draws, _) = array(&original, meshes + 24).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            &paired.payload[meshes + 0x28..meshes + 0x88],
            &original[meshes + 0x28..meshes + 0x88]
        );
        assert_eq!(read_u32(&paired.payload, draws).unwrap(), u32::MAX);
        assert!(!paired.patches.iter().any(|(at, _)| *at == draws));
        for index in 0..3 {
            let at = draws + index * 32;
            assert_eq!(&paired.payload[at + 4..at + 8], &original[at + 4..at + 8]);
            assert_eq!(
                &paired.payload[at + 20..at + 32],
                &original[at + 20..at + 32]
            );
            assert_eq!(read_u32(&paired.payload, at + 8).unwrap(), 0);
            assert_eq!(read_u32(&paired.payload, at + 12).unwrap(), 9);
            assert_eq!(read_u32(&paired.payload, at + 16).unwrap(), 4);
            assert_eq!(
                paired
                    .patches
                    .iter()
                    .filter(|(field, _)| *field == at)
                    .count(),
                usize::from(index != 0)
            );
        }
        let positions = nodes
            .iter()
            .find(|node| node.template.0 == 0x80BC5C73)
            .unwrap();
        assert_eq!(positions.payload.len(), 64);
        let indices = nodes
            .iter()
            .find(|node| node.template.0 == 0x80C231EE)
            .unwrap();
        assert_eq!(indices.payload.len(), 18);
        println!(
            "Native absent slot retained; two real materials transformed;4 vertices become8;shared strip becomes9indices;stage/LOD metadata exact"
        );
    }

    #[test]
    #[ignore = "requires clean SUNDIAL_STOCK_PACKAGES; reads stock assets without installing"]
    fn akimbo_stock_two_bone_model_preserves_native_weights_and_remaps_both_hands() {
        let packages = crate::test_support::stock_packages();
        sundial::package_authoring::initialize_source_decoder(&packages).unwrap();
        let manager =
            sundial::package_authoring::open_shadowkeep_package_manager(&packages).unwrap();
        let pose = crate::weapon::akimbo::visual::pose::prepare(
            &read(&manager, 0x80C1DEA7).unwrap(),
            &read(&manager, 0x80BC5EE4).unwrap(),
            |tag| read(&manager, tag),
        )
        .unwrap();
        let original_model = read(&manager, 0x80C68962).unwrap();
        let original = read(&manager, 0x8152304B).unwrap();
        let mut paired = Node::new("weighted-stock-model", 0x80C68962, original_model.clone());
        let mut nodes = Vec::new();
        assert!(
            model(
                &manager,
                "weighted-stock",
                &mut paired,
                &mut nodes,
                false,
                0.36,
                Some(&pose)
            )
            .unwrap()
        );
        let generated = nodes
            .iter()
            .filter(|node| node.template.0 == 0x8152304B)
            .collect::<Vec<_>>();
        let [generated] = generated.as_slice() else {
            panic!("Expected one converted packed vertex stream")
        };
        assert_eq!(generated.payload.len(), original.len() * 2);
        let native_count = original.len() / 12;
        let source_scale = f32::from_bits(read_u32(&original_model, 0x6C).unwrap()) as f64;
        let final_scale = f32::from_bits(read_u32(&paired.payload, 0x6C).unwrap()) as f64;
        let mut maximum_error = 0f64;
        for (index, row) in generated.payload.chunks_exact(12).enumerate() {
            let source = &original[(index % native_count) * 12..(index % native_count + 1) * 12];
            let left = index >= native_count;
            assert_eq!(
                read_u16(row, 6).unwrap(),
                if left { geometry::LEFT_COPY_FLAGS } else { 0 }
            );
            assert_eq!(&row[10..12], &source[10..12]);
            for lane in [8, 9] {
                assert_eq!(
                    usize::from(row[lane]),
                    pose.renumbering.map[usize::from(source[lane])]
                );
            }
            for axis in 0..3 {
                let decode = |bytes: &[u8], scale| {
                    i16::from_le_bytes(bytes[axis * 2..axis * 2 + 2].try_into().unwrap()) as f64
                        / 32767.0
                        * scale
                };
                let expected = decode(source, source_scale)
                    + if left && axis == 1 {
                        f64::from(0.36f32)
                    } else {
                        0.0
                    };
                maximum_error = maximum_error.max((decode(row, final_scale) - expected).abs());
            }
        }
        assert!(maximum_error <= 0.0001);
        let attributes = nodes
            .iter()
            .find(|node| node.template.0 == 0x8152304C)
            .unwrap();
        assert_eq!(
            attributes.payload,
            read(&manager, 0x8152304C).unwrap().repeat(2)
        );
        println!(
            "Validated {native_count} native packed vertices into {} paired vertices; maximum position error {maximum_error}; weights and attributes exact",
            native_count * 2
        );
    }

    #[test]
    fn akimbo_material_palette_follows_its_indexed_vertices() {
        let positions = [0u16, 7, 2, 4]
            .into_iter()
            .flat_map(|bone| {
                let mut vertex = [0; 8];
                vertex[6..8].copy_from_slice(&bone.to_le_bytes());
                vertex
            })
            .collect::<Vec<_>>();
        let mut draw = geometry::Draw {
            first_index: 1,
            index_count: 4,
            primitive_count: 1,
            topology: geometry::Topology::TriangleStrip,
            duplicate: true,
        };
        let indices = [1, 0, 2, u16::MAX, 3, 1];
        assert_eq!(draw_palette(&positions, &indices, &draw, 8).unwrap(), 5);
        draw.first_index = 0;
        assert_eq!(draw_palette(&positions, &indices, &draw, 8).unwrap(), 8);
        draw.first_index = 3;
        draw.index_count = 1;
        assert!(draw_palette(&positions, &indices, &draw, 8).is_err());
        draw.first_index = 6;
        assert!(draw_palette(&positions, &indices, &draw, 8).is_err());
        draw.first_index = 0;
        assert!(draw_palette(&positions, &[4], &draw, 8).is_err());
    }

    #[test]
    fn akimbo_weighted_draw_palette_checks_both_influences_even_at_zero_weight() {
        let positions = [[0u8, 7, 255, 0], [2, 4, 127, 128], [1, 3, 0, 255]]
            .into_iter()
            .flat_map(|weights| [vec![0; 8], weights.to_vec()].concat())
            .collect::<Vec<_>>();
        let mut draw = geometry::Draw {
            first_index: 0,
            index_count: 3,
            primitive_count: 1,
            topology: geometry::Topology::TriangleList,
            duplicate: true,
        };
        assert_eq!(draw_palette(&positions, &[0, 1, 2], &draw, 12).unwrap(), 8);
        draw.first_index = 1;
        draw.index_count = 2;
        assert_eq!(draw_palette(&positions, &[0, 1, 2], &draw, 12).unwrap(), 5);
        assert!(draw_palette(&positions, &[0, 1, 3], &draw, 12).is_err());
    }
}
