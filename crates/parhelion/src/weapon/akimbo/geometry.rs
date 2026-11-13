//! Checked rigid and packed two-influence geometry for the single-marker pair protocol.
//!
//! This is only the geometry transformation. The caller must independently validate the
//! vertex/effect programs, the unused marker's animation/attachment roles, and the native pose
//! adapter before publishing a model. A successful result does not establish akimbo support.
//! All parts that share the model's quantization must be passed together, including parts such
//! as a central reticle that are retained once. Material, stage and LOD records stay with their
//! original draw; only its first index, index count and primitive count change.

use std::collections::BTreeMap;

const POSITION_STRIDE: usize = 8;
const RESTART: u16 = u16::MAX;
const POSITION_UNIT: f64 = 32767.0;
/// Both bits belong to the single-marker shader protocol, never to native bone indices.
pub const LEFT_COPY_FLAGS: u16 = 0xC0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Topology {
    TriangleList,
    TriangleStrip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Draw {
    pub first_index: u32,
    pub index_count: u32,
    pub primitive_count: u32,
    pub topology: Topology,
    /// Pair this draw when its containing part is duplicated. A retained central reticle
    /// can share the native streams with paired geometry without drawing a second reticle.
    pub duplicate: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    /// Eight-byte rigid rows, or twelve-byte packed two-influence rows. In the latter,
    /// bytes6..8 are an unused zero copy lane and bytes8..12 retain bones0/1, weights0/1.
    pub positions: &'a [u8],
    pub position_stride: u16,
    /// Attribute bytes, including UVs, normals and tangents, are copied without alteration.
    pub attributes: &'a [u8],
    pub attribute_stride: u16,
    pub indices: &'a [u16],
    pub draws: &'a [Draw],
    pub duplicate: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub position_scale: f32,
    pub position_offset: [f32; 3],
    pub bone_palette: u16,
    pub parts: &'a [Part<'a>],
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Baked left-copy translation on weapon-local Y and the shader's inverse pivot.
    /// The native pose centers the pair at -gap/2; applying that shift here as well would
    /// double-center the mesh and detach its muzzle, ejection markers and animated bones.
    pub separation: f32,
    pub marker_bone: u16,
    /// Maximum allowed position error after common quantization, in model-space units.
    pub max_position_error: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedPart {
    pub positions: Vec<u8>,
    pub attributes: Vec<u8>,
    pub indices: Vec<u16>,
    /// One result per input draw in exactly the input order. Draw identities do not grow.
    pub draws: Vec<Draw>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedGeometry {
    /// Apply to both scalar quantization and the descriptor's matching XYZ scale fields.
    pub position_scale: f32,
    pub position_offset: [f32; 3],
    /// An enclosing sphere around `position_offset`, including retained parts and both copies.
    pub sphere_radius: f32,
    pub max_position_error: f64,
    pub parts: Vec<PreparedPart>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    InvalidTransform,
    InvalidSeparation,
    InvalidErrorLimit,
    InvalidPalette,
    InvalidMarker,
    EmptyModel,
    NoDuplicatedPart,
    UnsupportedPositionLayout,
    InvalidAttributeLayout,
    InvalidBoneLane,
    InvalidWeights,
    OccupiedMarker,
    TooManyVertices,
    InvalidIndex,
    EmptyDraw,
    InvalidDrawRange,
    OverlappingDrawRanges,
    UnownedIndices,
    ConflictingTopology,
    ConflictingDuplication,
    InvalidTriangleList,
    InvalidPrimitiveCount,
    OutputLimit,
    QuantizationError,
}

impl std::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Akimbo geometry: {self:?}")
    }
}

impl std::error::Error for GeometryError {}

type Result<T> = std::result::Result<T, GeometryError>;

fn vertices(part: Part<'_>) -> impl Iterator<Item = ([i16; 3], u16)> {
    part.positions
        .chunks_exact(usize::from(part.position_stride))
        .map(|row| {
            (
                std::array::from_fn(|axis| i16::from_le_bytes([row[axis * 2], row[axis * 2 + 1]])),
                u16::from_le_bytes([row[6], row[7]]),
            )
        })
}

fn relative_position(source: Source<'_>, quantized: [i16; 3], shift: f64) -> [f64; 3] {
    std::array::from_fn(|axis| {
        f64::from(quantized[axis]) / POSITION_UNIT * f64::from(source.position_scale)
            + if axis == 1 { shift } else { 0.0 }
    })
}

fn validate_source(source: Source<'_>, settings: Settings) -> Result<()> {
    if !source.position_scale.is_finite()
        || source.position_scale <= 0.0
        || source.position_offset.iter().any(|v| !v.is_finite())
    {
        return Err(GeometryError::InvalidTransform);
    }
    if !settings.separation.is_finite()
        || !(0.0..=1.0).contains(&settings.separation)
        || settings.separation == 0.0
    {
        return Err(GeometryError::InvalidSeparation);
    }
    if !settings.max_position_error.is_finite() || settings.max_position_error <= 0.0 {
        return Err(GeometryError::InvalidErrorLimit);
    }
    if source.bone_palette == 0 || source.bone_palette > 64 {
        return Err(GeometryError::InvalidPalette);
    }
    if settings.marker_bone == 0 || settings.marker_bone >= source.bone_palette {
        return Err(GeometryError::InvalidMarker);
    }
    if source.parts.is_empty() || source.parts.len() > 256 {
        return Err(GeometryError::EmptyModel);
    }
    if !source
        .parts
        .iter()
        .any(|part| part.duplicate && part.draws.iter().any(|draw| draw.duplicate))
    {
        return Err(GeometryError::NoDuplicatedPart);
    }
    let mut output_bytes = 0usize;
    for part in source.parts {
        validate_part(*part, source.bone_palette, settings.marker_bone)?;
        let multiplier = if part.duplicate { 2 } else { 1 };
        // Per-part bounds above make these products safe. Bound the aggregate before allocation.
        output_bytes += (part.positions.len() + part.attributes.len()) * multiplier
            + part.indices.len() * 2 * multiplier
            + part.draws.len() * (size_of::<Draw>() + 2);
        if output_bytes > 64 * 1024 * 1024 {
            return Err(GeometryError::OutputLimit);
        }
    }
    Ok(())
}

fn validate_part(part: Part<'_>, palette: u16, marker: u16) -> Result<()> {
    let stride = usize::from(part.position_stride);
    if ![POSITION_STRIDE, 12].contains(&stride)
        || part.positions.is_empty()
        || !part.positions.len().is_multiple_of(stride)
    {
        return Err(GeometryError::UnsupportedPositionLayout);
    }
    let count = part.positions.len() / stride;
    let multiplier = if part.duplicate { 2 } else { 1 };
    if count > usize::from(RESTART) / multiplier {
        return Err(GeometryError::TooManyVertices);
    }
    let stride = usize::from(part.attribute_stride);
    if !(4..=128).contains(&stride) || part.attributes.len() != count * stride {
        return Err(GeometryError::InvalidAttributeLayout);
    }
    for (index, (_, bone)) in vertices(part).enumerate() {
        let row = &part.positions[index * usize::from(part.position_stride)..];
        let bones = if part.position_stride == 12 {
            if bone != 0 {
                return Err(GeometryError::InvalidBoneLane);
            }
            if u16::from(row[10]) + u16::from(row[11]) != 255 {
                return Err(GeometryError::InvalidWeights);
            }
            [u16::from(row[8]), u16::from(row[9])]
        } else {
            [bone, bone]
        };
        for bone in bones {
            // High native indices must be refused, never truncated with `& 0x3f`.
            if bone >= palette || bone >= 64 {
                return Err(GeometryError::InvalidBoneLane);
            }
            if bone == marker {
                return Err(GeometryError::OccupiedMarker);
            }
        }
    }
    if part.indices.len() > 16 * 1024 * 1024 || part.draws.len() > 65536 {
        return Err(GeometryError::OutputLimit);
    }
    if part
        .indices
        .iter()
        .any(|&index| index != RESTART && usize::from(index) >= count)
    {
        return Err(GeometryError::InvalidIndex);
    }
    validate_draws(part)?;
    Ok(())
}

fn primitive_bounds(run: &[u16], topology: Topology) -> Result<(u32, u32)> {
    let mut visible = 0;
    let mut total = 0;
    match topology {
        Topology::TriangleList => {
            if !run.len().is_multiple_of(3) || run.contains(&RESTART) {
                return Err(GeometryError::InvalidTriangleList);
            }
            for tri in run.chunks_exact(3) {
                visible += u32::from(tri[0] != tri[1] && tri[0] != tri[2] && tri[1] != tri[2]);
                total += 1;
            }
        }
        Topology::TriangleStrip => {
            for strip in run.split(|index| *index == RESTART) {
                for tri in strip.windows(3) {
                    visible += u32::from(tri[0] != tri[1] && tri[0] != tri[2] && tri[1] != tri[2]);
                    total += 1;
                }
            }
        }
    }
    Ok((visible, total))
}

fn validate_draws(part: Part<'_>) -> Result<()> {
    if part.draws.is_empty() {
        return Err(GeometryError::EmptyDraw);
    }
    let mut ranges = BTreeMap::new();
    for draw in part.draws {
        if draw.index_count == 0 {
            return Err(GeometryError::EmptyDraw);
        }
        let end = draw
            .first_index
            .checked_add(draw.index_count)
            .ok_or(GeometryError::InvalidDrawRange)?;
        let run = part
            .indices
            .get(draw.first_index as usize..end as usize)
            .ok_or(GeometryError::InvalidDrawRange)?;
        let bounds = primitive_bounds(run, draw.topology)?;
        // Shipped models sometimes count some degenerate triangles; preserve that convention.
        if !(bounds.0..=bounds.1).contains(&draw.primitive_count) {
            return Err(GeometryError::InvalidPrimitiveCount);
        }
        if let Some((topology, duplicate)) =
            ranges.insert((draw.first_index, end), (draw.topology, draw.duplicate))
        {
            if topology != draw.topology {
                return Err(GeometryError::ConflictingTopology);
            }
            if duplicate != draw.duplicate {
                return Err(GeometryError::ConflictingDuplication);
            }
        }
    }
    let mut cursor = 0;
    for ((first, end), _) in ranges {
        if first < cursor {
            return Err(GeometryError::OverlappingDrawRanges);
        }
        if part.indices[cursor as usize..first as usize]
            .iter()
            .any(|index| *index != RESTART)
        {
            return Err(GeometryError::UnownedIndices);
        }
        cursor = end;
    }
    if cursor as usize != part.indices.len() {
        return Err(GeometryError::UnownedIndices);
    }
    Ok(())
}

fn prepare_indices(part: Part<'_>) -> Result<(Vec<u16>, Vec<Draw>)> {
    if !part.duplicate {
        return Ok((part.indices.to_vec(), part.draws.to_vec()));
    }
    let count = (part.positions.len() / usize::from(part.position_stride)) as u16;
    let ranges = part
        .draws
        .iter()
        .map(|draw| {
            (
                (draw.first_index, draw.index_count),
                (draw.topology, draw.duplicate),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::new();
    let mut replacements = BTreeMap::new();
    let mut cursor = 0;
    for ((first, length), (topology, duplicate)) in ranges {
        let end = first + length;
        output.extend_from_slice(&part.indices[cursor as usize..first as usize]);
        let new_first = u32::try_from(output.len()).map_err(|_| GeometryError::OutputLimit)?;
        let run = &part.indices[first as usize..end as usize];
        output.extend_from_slice(run);
        // Restart resets strip winding and prevents a triangle joining the two hands.
        if duplicate {
            if topology == Topology::TriangleStrip {
                output.push(RESTART);
            }
            output.extend(run.iter().map(|&index| {
                if index == RESTART {
                    RESTART
                } else {
                    index + count
                }
            }));
        }
        let new_length =
            u32::try_from(output.len()).map_err(|_| GeometryError::OutputLimit)? - new_first;
        replacements.insert((first, length), (new_first, new_length));
        cursor = end;
    }
    let mut draws = Vec::with_capacity(part.draws.len());
    for draw in part.draws {
        let &(first_index, index_count) = replacements
            .get(&(draw.first_index, draw.index_count))
            .ok_or(GeometryError::InvalidDrawRange)?;
        draws.push(Draw {
            first_index,
            index_count,
            primitive_count: draw
                .primitive_count
                .checked_mul(if draw.duplicate { 2 } else { 1 })
                .ok_or(GeometryError::OutputLimit)?,
            topology: draw.topology,
            duplicate: draw.duplicate,
        });
    }
    Ok((output, draws))
}

fn dimensions(source: Source<'_>, settings: Settings) -> Result<(f32, f32)> {
    let mut extent = f64::from(source.position_scale);
    let mut radius_squared: f64 = 0.0;
    for &part in source.parts {
        for (quantized, _) in vertices(part) {
            for copy in 0..=usize::from(part.duplicate) {
                let point = relative_position(
                    source,
                    quantized,
                    copy as f64 * f64::from(settings.separation),
                );
                let mut length = 0.0;
                for value in point {
                    extent = extent.max(value.abs());
                    length += value * value;
                }
                radius_squared = radius_squared.max(length);
            }
        }
    }
    // Headroom also ensures f64 -> f32 rounding cannot shrink below a required endpoint.
    let scale = (extent * 1.0001) as f32;
    // Include the maximum quantization displacement on all three axes.
    let radius = (radius_squared.sqrt() + f64::from(scale) / POSITION_UNIT) * 1.0001;
    if !scale.is_finite() || scale <= 0.0 || !(radius as f32).is_finite() {
        return Err(GeometryError::InvalidTransform);
    }
    Ok((scale, radius as f32))
}

/// Duplicate every selected part from native data, using one quantization domain for the model.
/// No source buffers are mutated. Every validation precedes returning publishable output.
pub fn prepare(source: Source<'_>, settings: Settings) -> Result<PreparedGeometry> {
    validate_source(source, settings)?;
    let (scale, sphere_radius) = dimensions(source, settings)?;
    let mut maximum_error: f64 = 0.0;
    let mut parts = Vec::with_capacity(source.parts.len());
    for &part in source.parts {
        let mut positions =
            Vec::with_capacity(part.positions.len() * if part.duplicate { 2 } else { 1 });
        for copy in 0..=usize::from(part.duplicate) {
            for (index, (quantized, bone)) in vertices(part).enumerate() {
                let point = relative_position(
                    source,
                    quantized,
                    copy as f64 * f64::from(settings.separation),
                );
                for relative in point {
                    let encoded = (relative / f64::from(scale) * POSITION_UNIT).round_ties_even();
                    if !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&encoded) {
                        return Err(GeometryError::QuantizationError);
                    }
                    let decoded = encoded / POSITION_UNIT * f64::from(scale);
                    maximum_error = maximum_error.max((decoded - relative).abs());
                    positions.extend_from_slice(&(encoded as i16).to_le_bytes());
                }
                positions.extend_from_slice(
                    &(bone | if copy == 0 { 0 } else { LEFT_COPY_FLAGS }).to_le_bytes(),
                );
                if part.position_stride == 12 {
                    positions.extend_from_slice(&part.positions[index * 12 + 8..index * 12 + 12]);
                }
            }
        }
        let mut attributes = part.attributes.to_vec();
        if part.duplicate {
            attributes.extend_from_slice(part.attributes);
        }
        let (indices, draws) = prepare_indices(part)?;
        parts.push(PreparedPart {
            positions,
            attributes,
            indices,
            draws,
        });
    }
    if maximum_error > f64::from(settings.max_position_error) {
        return Err(GeometryError::QuantizationError);
    }
    Ok(PreparedGeometry {
        position_scale: scale,
        position_offset: source.position_offset,
        sphere_radius,
        max_position_error: maximum_error,
        parts,
    })
}

/// Native draw masks use 32-bit words. This does not allocate or mutate model mask arrays.
pub fn record_mask_capacity(record_ids: &[u16]) -> u32 {
    record_ids
        .iter()
        .max()
        .map_or(0, |&id| (u32::from(id) + 1).div_ceil(32) * 32)
}

#[cfg(test)]
#[path = "geometry/tests.rs"]
mod tests;
