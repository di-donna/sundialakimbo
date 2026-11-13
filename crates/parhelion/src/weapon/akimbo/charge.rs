//! Experimental serialized charge storage. Not admitted by the compiler or native host.
//!
//! Native CF16F0 initializes three properties whose handles F40240 binds to an offset in
//! the weapon's allocation. A heap copy is not relocatable with that allocation. This
//! helper grows the serialized instance region instead; no initialized controller or
//! effect handle is copied. Initialization, teardown and hand-qualified effect queries
//! still need a production bridge before an emitter may use this payload.

use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 16 * 1024 * 1024;
// Reflected build86657 metadata gives these complete padded sizes. The final
// definition field at +410 is the optional sequence; it must remain absent.
const INSTANCE: usize = 0x220;
const DEFINITION: usize = 0x418;
const DELTA: usize = 0x230;
const EXPRESSIONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub instance: usize,
    pub definition: usize,
    pub root_definition: usize,
    pub mode: u32,
}

#[derive(Debug)]
pub(crate) struct Prepared {
    pub bytes: Vec<u8>,
    pub original: Layout,
    pub appended: Layout,
    /// External typed entity references at or beyond `cut` must follow this shift.
    pub cut: usize,
    pub delta: usize,
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> Result<[u8; N]> {
    b.get(at..at.checked_add(N).ok_or("Charge offset overflow")?)
        .ok_or_else(|| format!("Charge field 0x{at:X} is truncated"))?
        .try_into()
        .map_err(|_| "Charge field width differs".into())
}
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(b, at)?))
}
fn u64_at(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes(b, at)?))
}
fn relative(b: &[u8], at: usize) -> Result<usize> {
    at.checked_add_signed(i64::from_le_bytes(bytes(b, at)?) as isize)
        .filter(|&v| v < b.len())
        .ok_or_else(|| format!("Charge pointer 0x{at:X} leaves its owner"))
}
fn put32(b: &mut [u8], at: usize, value: u32) {
    b[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, value: u64) {
    b[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn pointer(b: &mut [u8], at: usize, target: usize) {
    put64(b, at, (target as i64 - at as i64) as u64);
}

fn pair(b: &[u8], tag: u32, at: usize, kind: u32, definition: usize, class: u32) -> Result<()> {
    if u32_at(b, at)? != tag
        || u32_at(b, at + 4)? != kind
        || u64_at(b, at + 8)? != definition as u64
        || u32_at(b, definition)? != tag
        || u32_at(b, definition + 4)? != class
        || u64_at(b, definition + 8)? != at as u64
    {
        return Err(format!("Charge typed ownership differs at 0x{at:X}"));
    }
    Ok(())
}

pub(crate) fn inspect(b: &[u8], tag: u32, at: usize) -> Result<Layout> {
    if b.len() > LIMIT
        || b.len() < 0x3000
        || u64_at(b, 0)? != b.len() as u64
        || relative(b, 0x10)? != 0x80
    {
        return Err("Charge component owner size or instance header differs".into());
    }
    let root = relative(b, 0x18)?;
    if root < 0x27B0
        || root % 8 != 0
        || u64_at(b, 0x48)? != (root - 0x80) as u64
        || u64_at(b, 0x88)? != root as u64
        || at < 0x27B0
        || !at.is_multiple_of(8)
        || at.checked_add(INSTANCE).is_none_or(|end| end > root)
    {
        return Err("Charge controller is outside the native instance region".into());
    }
    let definition =
        usize::try_from(u64_at(b, at + 8)?).map_err(|_| "Charge definition overflow")?;
    if definition < root
        || definition % 8 != 0
        || definition
            .checked_add(DEFINITION)
            .is_none_or(|end| end > b.len())
    {
        return Err("Charge definition is outside its owner".into());
    }
    pair(b, tag, at, 0x80803ABE, definition, 0x80803ABD)?;
    if relative(b, at + 0x10)? != 0xCD0
        || u64_at(b, at + 0x18)? != 0
        || u64_at(b, at + 0x210)? != 0
        || u64_at(b, definition + 0x410)? != 0
        || u64_at(b, at + 0x218)? != 0
    {
        return Err("Charge parent or optional sequence differs".into());
    }
    let mode = u32_at(b, definition + 0x2E8)?;
    if ![2, 3].contains(&mode) {
        return Err("Charge mode is outside the qualified fusion/bow source shape".into());
    }
    for i in 0..EXPRESSIONS {
        let child = at + 0xB0 + i * 0x30;
        pair(
            b,
            tag,
            child,
            0x808089F6,
            definition + 0xA8 + i * 0x60,
            0x808089F5,
        )?;
        if relative(b, child + 0x10)? != at {
            return Err("Charge expression has a different parent".into());
        }
        if u64_at(b, child + 0x18)? != 0 {
            return Err("Charge source contains initialized expression state".into());
        }
        let code = array(b, definition + 0xA8 + i * 0x60 + 0x10, 0x80800009, 1, 8192)?;
        let constants = array(b, definition + 0xA8 + i * 0x60 + 0x20, 0x80800090, 16, 4096)?;
        if code.is_empty() || code.iter().chain(constants.iter()).any(|&row| row < root) {
            return Err("Charge expression program leaves the immutable region".into());
        }
        providers(b, tag, child, definition + 0xA8 + i * 0x60, root)?;
    }
    for field in [0x10, 0x30, 0x50] {
        let property = relative(b, definition + field)?;
        if property < root
            || property % 8 != 0
            || property.checked_add(24).is_none_or(|end| end > b.len())
        {
            return Err("Charge property definition is outside its owner".into());
        }
    }
    // Only pristine serialized state is a template. Do not accept a captured live
    // property list, effect token, elapsed state or selected-content override.
    for i in 0..3 {
        let property = at + 0x20 + i * 0x30;
        if u64_at(b, property)? != 0
            || u64_at(b, property + 8)? != 0
            || u64_at(b, property + 16)? != 0x00000000FFFFFFFF
            || u64_at(b, property + 24)? != 0x00000000FFFFFFFF
            || u64_at(b, property + 32)? != u64::MAX
            || u64_at(b, property + 40)? != 0
        {
            return Err("Charge source contains initialized native property state".into());
        }
    }
    if u64_at(b, at + 0x1D0)? != 0
        || u64_at(b, at + 0x1D8)? != u64::MAX
        || u64_at(b, at + 0x1E0)? != 0
        || u64_at(b, at + 0x1E8)? != 0
        || u64_at(b, at + 0x1F0)? != u64::MAX
        || u64_at(b, at + 0x1F8)? != 0
        || u32_at(b, at + 0x200)? != 0
        || u64_at(b, at + 0x204)? != u64::MAX
        || u32_at(b, at + 0x20C)? != u32::MAX
    {
        return Err("Charge source contains live state or an owned effect".into());
    }
    Ok(Layout {
        instance: at,
        definition,
        root_definition: root,
        mode,
    })
}

#[derive(Clone, Copy)]
struct Provider {
    state: usize,
    definition: usize,
}

fn array(b: &[u8], at: usize, class: u32, stride: usize, maximum: usize) -> Result<Vec<usize>> {
    let count = usize::try_from(u64_at(b, at)?).map_err(|_| "Charge array count overflow")?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let h = relative(b, at + 8)?;
    if count > maximum
        || h < 4
        || u32_at(b, h - 4)? != 0x80809FBD
        || u64_at(b, h)? != count as u64
        || u64_at(b, h + 8)? != u64::from(class)
        || h.checked_add(16 + count * stride)
            .is_none_or(|end| end > b.len())
    {
        return Err("Charge expression array has a different native shape".into());
    }
    Ok((0..count).map(|i| h + 16 + i * stride).collect())
}

fn providers(
    b: &[u8],
    tag: u32,
    instance: usize,
    definition: usize,
    root: usize,
) -> Result<Vec<Provider>> {
    let states = array(b, instance + 0x20, 0x80809788, 96, 32)?;
    let definitions = array(b, definition + 0x40, 0x80809789, 40, 32)?;
    if states.len() != definitions.len() {
        return Err("Charge provider arrays disagree".into());
    }
    let mut result = Vec::new();
    for (s, d) in states.into_iter().zip(definitions) {
        if s < 0x80 || s + 96 > root || d < root || relative(b, s + 16)? != instance {
            return Err("Charge provider is outside its owning region".into());
        }
        pair(b, tag, s, 0x80809789, d, 0x80809788)?;
        // Native named-channel providers have a fixed pristine state, including
        // absent weak handles. Dynamic bindings are reconstructed by the host.
        for (field, value) in [
            (0x18, 0),
            (0x20, u64::MAX),
            (0x28, 0),
            (0x30, u64::from(u32::MAX)),
            (0x38, u64::MAX),
            (0x40, 0),
            (0x48, 0),
            (0x50, 0),
            (0x58, 0),
        ] {
            if u64_at(b, s + field)? != value {
                return Err("Charge provider contains initialized state".into());
            }
        }
        if u64_at(b, d + 16)? != 0 || u64_at(b, d + 24)? != 0x808097C1 || u32_at(b, d + 36)? != 0 {
            return Err("Charge provider uses an unqualified property route".into());
        }
        result.push(Provider {
            state: s,
            definition: d,
        });
    }
    Ok(result)
}

fn append_array(out: &mut Vec<u8>, class: u32, rows: &[u8], stride: usize) -> usize {
    let first = (out.len() + 0x23) & !15;
    out.resize(first, 0);
    put32(out, first - 20, 0x80809FBD);
    put64(out, first - 16, (rows.len() / stride) as u64);
    put64(out, first - 8, u64::from(class));
    out.extend_from_slice(rows);
    first - 16
}

// Match both the typed address and its target's native type. Equal opaque words
// without that target marker/header are not relocated.
fn typed(b: &[u8], tag: u32) -> Result<BTreeSet<usize>> {
    let mut fields = BTreeSet::new();
    for at in (0..b.len().saturating_sub(15)).step_by(8) {
        if u32_at(b, at)? != tag {
            continue;
        }
        let class = u32_at(b, at + 4)?;
        if class < 0x80000000 || class == u32::MAX {
            continue;
        }
        let Ok(target) = usize::try_from(u64_at(b, at + 8)?) else {
            continue;
        };
        let reciprocal = u32_at(b, target).ok() == Some(tag)
            && u32_at(b, target + 4).is_ok_and(|kind| kind >= 0x80000000 && kind != u32::MAX)
            && u64_at(b, target + 8).ok() == Some(at as u64);
        let interface = u64_at(b, target + 8)
            .ok()
            .and_then(|value| usize::try_from(value).ok())
            .is_some_and(|definition| {
                u32_at(b, target).ok() == Some(tag)
                    && u32_at(b, definition).ok() == Some(tag)
                    && u32_at(b, definition + 4).ok() == Some(class)
                    && u64_at(b, definition + 8).ok() == Some(target as u64)
            });
        if target % 8 == 0
            && target < b.len()
            && (target >= 4 && u32_at(b, target - 4).ok() == Some(class)
                || u32_at(b, target + 4).ok() == Some(class)
                || reciprocal
                || interface)
        {
            fields.insert(at);
        }
    }
    Ok(fields)
}

fn header_pointers(b: &[u8], root: usize) -> Result<BTreeSet<usize>> {
    let mut fields = BTreeSet::from([0x18, 0x28, 0x38]);
    for (at, stride, class) in [(0x20, 8, 0x8080919Fu32), (0x30, 16, 0x808091A4)] {
        let count =
            usize::try_from(u64_at(b, at)?).map_err(|_| "Charge header table is too large")?;
        let header = relative(b, at + 8)?;
        if count > 4096
            || header < root
            || header < 4
            || u32_at(b, header - 4)? != 0x80809FBD
            || u64_at(b, header)? != count as u64
            || u64_at(b, header + 8)? != u64::from(class)
            || header
                .checked_add(16 + count * stride)
                .is_none_or(|end| end > b.len())
        {
            return Err("Charge native relocation/dispatch table differs".into());
        }
        for i in 0..count {
            let field = header + 16 + i * stride;
            let target = relative(b, field)?;
            if !(0x80..root).contains(&target) {
                return Err("Charge header table leaves its instance region".into());
            }
            fields.insert(field);
        }
    }
    Ok(fields)
}

/// Prepare static storage only. The original trigger still points to its original
/// controller; an unqualified native host cannot discover or activate the new one.
/// The caller must also rebase its entity's typed core references and qualify the
/// allocation/replication metadata before installing this experimental payload.
pub(crate) fn prepare(b: &[u8], tag: u32, at: usize) -> Result<Prepared> {
    let layout = inspect(b, tag, at)?;
    if relative(b, 0xCD0 + 0xF0)? != at {
        return Err("Charge trigger does not select its source controller".into());
    }
    let cut = layout.root_definition - 16;
    // Preserve the old root definition's type marker with the moved definition.
    if b.get(cut..cut + 12) != Some(&[0; 12]) || u32_at(b, cut + 12)? != u32_at(b, 0x84)? {
        return Err("Charge root definition has no isolated marker/padding boundary".into());
    }
    let groups = (0..EXPRESSIONS)
        .map(|i| {
            providers(
                b,
                tag,
                at + 0xB0 + i * 0x30,
                layout.definition + 0xA8 + i * 0x60,
                layout.root_definition,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut region = vec![0; DELTA];
    let mut state_headers = Vec::new();
    for group in &groups {
        state_headers.push(if group.is_empty() {
            None
        } else {
            let rows = group
                .iter()
                .flat_map(|p| b[p.state..p.state + 96].iter().copied())
                .collect::<Vec<_>>();
            Some(append_array(&mut region, 0x80809788, &rows, 96))
        });
    }
    region.resize((region.len() + 15) & !15, 0);
    let delta = region.len();
    let fields = typed(b, tag)?;
    let pointers = header_pointers(b, layout.root_definition)?;
    let skip = fields
        .iter()
        .flat_map(|&p| [p, p + 8])
        .collect::<BTreeSet<_>>();
    // Unknown crossing words are a refusal, never a guessed pointer rewrite.
    for field in (0..b.len().saturating_sub(7)).step_by(8) {
        if skip.contains(&field) || field == 0x48 {
            continue;
        }
        if let Ok(target) = relative(b, field)
            && target.is_multiple_of(8)
            && (field < cut) != (target < cut)
            && !pointers.contains(&field)
        {
            return Err(format!(
                "Charge owner has an unqualified crossing field at 0x{field:X}"
            ));
        }
    }
    let shifted = |offset: usize| {
        if offset >= cut {
            offset + delta
        } else {
            offset
        }
    };
    let mut out = Vec::with_capacity(b.len() + delta + DEFINITION + 32);
    out.extend_from_slice(&b[..cut]);
    out.extend_from_slice(&region);
    out.extend_from_slice(&b[cut..]);
    for &field in &fields {
        let target =
            usize::try_from(u64_at(b, field + 8)?).map_err(|_| "Charge typed target overflow")?;
        put64(&mut out, shifted(field) + 8, shifted(target) as u64);
    }
    for &field in &pointers {
        pointer(&mut out, shifted(field), shifted(relative(b, field)?));
    }
    put64(&mut out, 0x48, u64_at(b, 0x48)? + delta as u64);
    let second = cut + 16;
    put32(&mut out, second - 4, 0x80803ABD);
    out[second..second + INSTANCE].copy_from_slice(&b[at..at + INSTANCE]);
    pointer(&mut out, second + 0x10, 0xCD0);

    let definition = (out.len() + 19) & !15;
    out.resize((definition + DEFINITION + 15) & !15, 0);
    put32(&mut out, definition - 4, 0x80803ABE);
    out[definition..definition + DEFINITION]
        .copy_from_slice(&b[layout.definition..layout.definition + DEFINITION]);
    put64(&mut out, second + 8, definition as u64);
    put64(&mut out, definition + 8, second as u64);
    for i in 0..EXPRESSIONS {
        let child = second + 0xB0 + i * 0x30;
        let child_def = definition + 0xA8 + i * 0x60;
        put64(&mut out, child + 8, child_def as u64);
        put64(&mut out, child_def + 8, child as u64);
        pointer(&mut out, child + 0x10, second);
        // Immutable expression input arrays stay shared; an optional empty array's
        // unused pointer is preserved. No child expression is re-evaluated here.
        for descriptor in [0x10, 0x20] {
            let old = layout.definition + 0xA8 + i * 0x60 + descriptor;
            if u64_at(b, old)? != 0 {
                pointer(
                    &mut out,
                    child_def + descriptor + 8,
                    shifted(relative(b, old + 8)?),
                );
            }
        }
        if let Some(header) = state_headers[i] {
            let state_header = cut + header;
            let rows = groups[i]
                .iter()
                .flat_map(|p| b[p.definition..p.definition + 40].iter().copied())
                .collect::<Vec<_>>();
            let definition_header = append_array(&mut out, 0x80809789, &rows, 40);
            pointer(&mut out, child + 0x28, state_header);
            pointer(&mut out, child_def + 0x48, definition_header);
            for (j, _) in groups[i].iter().enumerate() {
                let state = state_header + 16 + j * 96;
                let target = definition_header + 16 + j * 40;
                put64(&mut out, state + 8, target as u64);
                put64(&mut out, target + 8, state as u64);
                pointer(&mut out, state + 16, child);
            }
        }
    }
    for field in [0x10, 0x30, 0x50] {
        pointer(
            &mut out,
            definition + field,
            shifted(relative(b, layout.definition + field)?),
        );
    }
    // Five feedback endpoints are relative to this definition, not to the old
    // definition. Preserve their binding keys and native dispatch metadata.
    for field in [0x370, 0x390, 0x3B0, 0x3D0, 0x3F0] {
        if relative(b, layout.definition + field)? != layout.definition {
            return Err("Charge feedback endpoint has a different source owner".into());
        }
        pointer(&mut out, definition + field, definition);
    }
    out.resize((out.len() + 15) & !15, 0);
    if out.len() > LIMIT {
        return Err("Appended charge owner exceeds size bound".into());
    }
    let length = out.len() as u64;
    put64(&mut out, 0, length);
    let original = inspect(&out, tag, at)?;
    let appended = inspect(&out, tag, second)?;
    if relative(&out, 0xCD0 + 0xF0)? != at || original.mode != appended.mode {
        return Err("Appended charge storage changed original trigger selection".into());
    }
    Ok(Prepared {
        bytes: out,
        original,
        appended,
        cut,
        delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const TAG: u32 = 0x81526485;
    const AT: usize = 0x2BF0;
    const ROOT: usize = 0x3200;
    const DEF: usize = 0x3500;
    fn fixture(mode: u32) -> Vec<u8> {
        let mut b = vec![0; 0x5000];
        put64(&mut b, 0, 0x5000);
        pointer(&mut b, 0x10, 0x80);
        pointer(&mut b, 0x18, ROOT);
        put32(&mut b, 0x80, TAG);
        put32(&mut b, 0x84, 0x80BFDFEB);
        put64(&mut b, 0x88, ROOT as u64);
        put32(&mut b, ROOT - 4, 0x80BFDFEB);
        put32(&mut b, ROOT, TAG);
        put32(&mut b, ROOT + 4, 0x80BFDFEA);
        put64(&mut b, ROOT + 8, 0x80);
        put64(&mut b, 0x48, (ROOT - 0x80) as u64);
        pointer(&mut b, 0xCD0 + 0xF0, AT);
        put32(&mut b, AT, TAG);
        put32(&mut b, AT + 4, 0x80803ABE);
        put64(&mut b, AT + 8, DEF as u64);
        pointer(&mut b, AT + 0x10, 0xCD0);
        put32(&mut b, DEF, TAG);
        put32(&mut b, DEF + 4, 0x80803ABD);
        put64(&mut b, DEF + 8, AT as u64);
        put32(&mut b, DEF + 0x2E8, mode);
        for i in 0..3 {
            let p = AT + 0x20 + i * 0x30;
            put64(&mut b, p + 16, u64::from(u32::MAX));
            put64(&mut b, p + 24, u64::from(u32::MAX));
            put64(&mut b, p + 32, u64::MAX);
            pointer(&mut b, DEF + 0x10 + i * 0x20, 0x4100 + i * 24);
        }
        for i in 0..6 {
            let at = AT + 0xB0 + i * 0x30;
            let d = DEF + 0xA8 + i * 0x60;
            put32(&mut b, at, TAG);
            put32(&mut b, at + 4, 0x808089F6);
            put64(&mut b, at + 8, d as u64);
            pointer(&mut b, at + 0x10, AT);
            put32(&mut b, d, TAG);
            put32(&mut b, d + 4, 0x808089F5);
            put64(&mut b, d + 8, at as u64);
            put64(&mut b, d + 0x10, 1);
            pointer(&mut b, d + 0x18, 0x4300 + i * 0x20);
            let h = 0x4300 + i * 0x20;
            put32(&mut b, h - 4, 0x80809FBD);
            put64(&mut b, h, 1);
            put64(&mut b, h + 8, 0x80800009);
        }
        for field in [0x1D8, 0x1F0, 0x204] {
            put64(&mut b, AT + field, u64::MAX);
        }
        put32(&mut b, AT + 0x20C, u32::MAX);
        for field in [0x370, 0x390, 0x3B0, 0x3D0, 0x3F0] {
            pointer(&mut b, DEF + field, DEF);
        }
        for (at, header, class) in [(0x20, 0x4D00, 0x8080919F), (0x30, 0x4E00, 0x808091A4)] {
            put64(&mut b, at, 1);
            pointer(&mut b, at + 8, header);
            put32(&mut b, header - 4, 0x80809FBD);
            put64(&mut b, header, 1);
            put64(&mut b, header + 8, class);
            pointer(&mut b, header + 16, 0x80);
        }
        b
    }
    fn with_provider() -> Vec<u8> {
        let mut b = fixture(3);
        let s = 0x2F50;
        let d = 0x4510;
        let expr = AT + 0xB0;
        let expr_def = DEF + 0xA8;
        for (descriptor, header, class) in [
            (expr + 0x20, s - 16, 0x80809788),
            (expr_def + 0x40, d - 16, 0x80809789),
        ] {
            put64(&mut b, descriptor, 1);
            pointer(&mut b, descriptor + 8, header);
            put32(&mut b, header - 4, 0x80809FBD);
            put64(&mut b, header, 1);
            put64(&mut b, header + 8, class);
        }
        put32(&mut b, s, TAG);
        put32(&mut b, s + 4, 0x80809789);
        put64(&mut b, s + 8, d as u64);
        pointer(&mut b, s + 16, expr);
        for (field, value) in [
            (0x20, u64::MAX),
            (0x30, u64::from(u32::MAX)),
            (0x38, u64::MAX),
        ] {
            put64(&mut b, s + field, value);
        }
        put32(&mut b, d, TAG);
        put32(&mut b, d + 4, 0x80809788);
        put64(&mut b, d + 8, s as u64);
        put64(&mut b, d + 24, 0x808097C1);
        put32(&mut b, d + 32, 0x7AA347DD);
        b
    }
    #[test]
    fn bow_provider_state_and_definition_are_copied_with_separate_typed_ownership() {
        let b = with_provider();
        let p = prepare(&b, TAG, AT).unwrap();
        let old = providers(
            &p.bytes,
            TAG,
            p.original.instance + 0xB0,
            p.original.definition + 0xA8,
            p.original.root_definition,
        )
        .unwrap();
        let new = providers(
            &p.bytes,
            TAG,
            p.appended.instance + 0xB0,
            p.appended.definition + 0xA8,
            p.appended.root_definition,
        )
        .unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(new.len(), 1);
        assert_ne!(old[0].state, new[0].state);
        assert_ne!(old[0].definition, new[0].definition);
        assert!(
            new[0].state >= p.appended.instance + INSTANCE
                && new[0].state + 96 <= p.appended.root_definition
        );
        assert_eq!(
            u32_at(&p.bytes, new[0].definition + 32).unwrap(),
            0x7AA347DD
        );
        assert!(p.delta > DELTA);
    }
    #[test]
    fn malformed_or_initialized_provider_descendants_refuse() {
        for (at, value) in [
            (0x2F50 + 0x20, 1),
            (0x2F50 + 16, 0),
            (0x4510 + 24, 0x808097C2),
            (DEF + 0xA8 + 0x40, 2),
            (AT + 0x210, 1),
            (DEF + 0x410, 1),
        ] {
            let mut b = with_provider();
            put64(&mut b, at, value);
            assert!(prepare(&b, TAG, AT).is_err(), "provider field {at:X}");
        }
    }
    #[test]
    fn typed_charge_storage_preserves_both_modes_and_original_selection() {
        for mode in [2, 3] {
            let b = fixture(mode);
            let p = prepare(&b, TAG, AT).unwrap();
            assert_eq!(p.original.definition, DEF + DELTA);
            assert_eq!(p.appended.instance, ROOT);
            assert_eq!((p.cut, p.delta), (ROOT - 16, DELTA));
            assert_eq!(p.appended.root_definition, ROOT + DELTA);
            assert_eq!(p.original.mode, p.appended.mode);
            assert_eq!(relative(&p.bytes, 0xCD0 + 0xF0).unwrap(), AT);
            assert_eq!(
                u64_at(&p.bytes, 0x48).unwrap(),
                (ROOT - 0x80 + DELTA) as u64
            );
            for i in 0..6 {
                let old =
                    relative(&p.bytes, p.original.definition + 0xA8 + i * 0x60 + 0x18).unwrap();
                let new =
                    relative(&p.bytes, p.appended.definition + 0xA8 + i * 0x60 + 0x18).unwrap();
                assert_eq!(old, new);
                assert_eq!(new, 0x4300 + i * 0x20 + DELTA);
            }
            for field in [0x370, 0x390, 0x3B0, 0x3D0, 0x3F0] {
                assert_eq!(
                    relative(&p.bytes, p.appended.definition + field).unwrap(),
                    p.appended.definition
                );
            }
        }
    }
    #[test]
    fn live_effect_property_and_foreign_children_are_never_templates() {
        for (field, value) in [
            (AT + 0x204, 0x123400001234),
            (AT + 0x20, 1),
            (AT + 0x1E0, 1),
            (AT + 0x210, 8),
            (AT + 0xC0, 0),
            (AT + 0x1D0, 3),
        ] {
            let mut b = fixture(2);
            put64(&mut b, field, value);
            assert!(prepare(&b, TAG, AT).is_err(), "field {field:X}");
        }
        for mode in [0, 1, 4, u32::MAX] {
            assert!(prepare(&fixture(mode), TAG, AT).is_err());
        }
    }
    #[test]
    fn relocation_preserves_equal_opaque_words_and_refuses_unknown_crossing() {
        let mut b = fixture(2);
        put32(&mut b, 0x280, TAG);
        put32(&mut b, 0x284, 0x80808888);
        put64(&mut b, 0x288, (ROOT + 128) as u64);
        // This opaque address-like word would be a crossing pointer, so it must
        // refuse rather than invent a relocation based on its numeric value.
        assert!(prepare(&b, TAG, AT).unwrap_err().contains("crossing"));
        put64(&mut b, 0x288, 0x100);
        let p = prepare(&b, TAG, AT).unwrap();
        assert_eq!(&p.bytes[0x280..0x290], &b[0x280..0x290]);
        let mut b = fixture(2);
        pointer(&mut b, 0x290, ROOT + 0x100);
        assert!(prepare(&b, TAG, AT).unwrap_err().contains("crossing"));
    }
    #[test]
    fn malformed_owner_region_and_feedback_refuse_before_output() {
        for field in [0x48, 0x18, 0x20, 0x4D08, DEF + 0x370] {
            let mut b = fixture(2);
            put64(&mut b, field, 0);
            assert!(prepare(&b, TAG, AT).is_err(), "field {field:X}");
        }
        let mut b = fixture(2);
        b[ROOT - 16] = 1;
        assert!(prepare(&b, TAG, AT).is_err());
        let mut b = fixture(2);
        pointer(&mut b, 0xCD0 + 0xF0, AT + 8);
        assert!(prepare(&b, TAG, AT).is_err());
    }
}
