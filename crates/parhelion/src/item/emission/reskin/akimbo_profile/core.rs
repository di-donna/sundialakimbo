//! Serialized ownership and instance topology required by the native firing adapter.
use crate::tag_payload::{read_i64, read_u32, read_u64, relative_target};
use crate::{AuthoringResult, error::invalid};

const COMPONENTS: [(usize, u32, u32, i64); 6] = [
    (0x80, 0x80BFDFEB, 0x80BFDFEA, 0),
    (0x6F0, 0x808043D8, 0x808043DA, -0x680),
    (0xCD0, 0x8080388B, 0x8080388A, -0xC60),
    (0xDF0, 0x80803865, 0x80803889, -0xD80),
    (0x2550, 0x8080384B, 0x80803A0F, -0x24E0),
    (0x2790, 0x80803845, 0x80803844, -0x2720),
];

// Typed interfaces observed in the stock gun graph. These describe ownership, not firing
// semantics: a valid tube, charge or continuous-fire graph still needs its corresponding host.
const ROOT_KINDS: [(u32, u32); 3] = [
    (0x80BFDFEB, 0x80BFDFEA),
    (0x80C1C533, 0x80C1C534),
    (0x80C22C04, 0x80FEB848),
];
const RELOAD_KINDS: [(u32, u32); 2] = [(0x80803845, 0x80803844), (0x80803843, 0x80803842)];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Layout {
    pub definitions: [usize; 6],
    pub root_kind: (u32, u32),
    pub reload_kind: (u32, u32),
    pub flags: u32,
    pub pattern_count: u64,
    pub delayed_emission: u32,
}

fn definition(bytes: &[u8], tag: u32, component: (usize, u32, u32, i64)) -> AuthoringResult<usize> {
    let (at, instance, class, parent) = component;
    let definition = usize::try_from(read_u64(bytes, at + 8)?)
        .map_err(|_| invalid("Akimbo native component definition exceeds addressable bytes"))?;
    if !definition.is_multiple_of(8)
        || definition < 0x27B0
        || definition
            .checked_add(16)
            .is_none_or(|end| end > bytes.len())
        || read_u32(bytes, at)? != tag
        || read_u32(bytes, at + 4)? != instance
        || read_i64(bytes, at + 16)? != parent
        || read_u32(bytes, definition)? != tag
        || read_u32(bytes, definition + 4)? != class
        || read_u64(bytes, definition + 8)? != at as u64
    {
        return Err(invalid(format!(
            "Akimbo native component at 0x{at:X} has different ownership, type, definition or parent"
        )));
    }
    Ok(definition)
}

pub(super) fn inspect(bytes: &[u8], tag: u32) -> AuthoringResult<Layout> {
    if bytes.len() > 16 * 1024 * 1024 || read_u64(bytes, 0)? != bytes.len() as u64 {
        return Err(invalid(
            "Akimbo native core has an unsupported declared size",
        ));
    }
    let kind = |at: usize, candidates: &[(u32, u32)]| {
        let instance = read_u32(bytes, at + 4)?;
        candidates
            .iter()
            .copied()
            .find(|pair| pair.0 == instance)
            .ok_or_else(|| {
                invalid(format!(
                    "Akimbo native component at 0x{at:X} has an unsupported instance type"
                ))
            })
    };
    let root_kind = kind(0x80, &ROOT_KINDS)?;
    let reload_kind = kind(0x2790, &RELOAD_KINDS)?;
    let mut components = COMPONENTS;
    components[0].1 = root_kind.0;
    components[0].2 = root_kind.1;
    components[5].1 = reload_kind.0;
    components[5].2 = reload_kind.1;
    let definitions: [usize; 6] = components
        .into_iter()
        .map(|value| definition(bytes, tag, value))
        .collect::<AuthoringResult<Vec<_>>>()?
        .try_into()
        .map_err(|_| invalid("Akimbo native component set is incomplete"))?;
    let root = definitions[0];
    let barrel = definitions[3];
    if root != relative_target(bytes, 0x18)? {
        return Err(invalid(
            "Akimbo native root disagrees with its component owner",
        ));
    }
    Ok(Layout {
        definitions,
        root_kind,
        reload_kind,
        flags: read_u32(bytes, barrel + 0xB70)?,
        pattern_count: read_u64(bytes, barrel + 0xE60)?,
        delayed_emission: read_u32(bytes, barrel + 0xEC8)?,
    })
}

pub(super) fn layout(bytes: &[u8], tag: u32) -> AuthoringResult<(u64, u64)> {
    let current = inspect(bytes, tag)?;
    // Adapter 7 is already deployed. Keep its contract unchanged; a source-derived gun layout
    // must use a new advertised adapter rather than silently requiring a newer schema-4 host.
    if current.root_kind != ROOT_KINDS[0]
        || current.reload_kind != RELOAD_KINDS[0]
        || current.definitions[4] != 0x6170
    {
        return Err(invalid(
            "Akimbo native root or magazine definition differs from the adapter",
        ));
    }
    if current.flags != 0x10 || current.pattern_count != 0 || current.delayed_emission != 0 {
        return Err(invalid(
            "Akimbo requires the audited single-projectile barrel with no spread pattern or delayed emission",
        ));
    }
    Ok((current.definitions[0] as u64, current.definitions[3] as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::{write_u32, write_u64};
    const TAG: u32 = 0x8152C7C3;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8880];
        let len = bytes.len() as u64;
        write_u64(&mut bytes, 0, len).unwrap();
        write_u64(&mut bytes, 0x18, 0x3460 - 0x18).unwrap();
        for ((at, instance, class, parent), definition) in COMPONENTS
            .into_iter()
            .zip([0x3460, 0x3890, 0x4320, 0x43B0, 0x6170, 0x63C0])
        {
            write_u32(&mut bytes, at, TAG).unwrap();
            write_u32(&mut bytes, at + 4, instance).unwrap();
            write_u64(&mut bytes, at + 8, definition as u64).unwrap();
            write_u64(&mut bytes, at + 16, parent as u64).unwrap();
            write_u32(&mut bytes, definition, TAG).unwrap();
            write_u32(&mut bytes, definition + 4, class).unwrap();
            write_u64(&mut bytes, definition + 8, at as u64).unwrap();
        }
        write_u32(&mut bytes, 0x43B0 + 0xB70, 0x10).unwrap();
        bytes
    }

    #[test]
    fn akimbo_core_checks_each_embedded_owner_kind_parent_and_definition() {
        let bytes = fixture();
        assert_eq!(layout(&bytes, TAG).unwrap(), (0x3460, 0x43B0));
        for (at, _, _, _) in COMPONENTS {
            for field in [0, 4, 16] {
                let mut changed = bytes.clone();
                changed[at + field] ^= 1;
                assert!(layout(&changed, TAG).is_err(), "instance {at:X}+{field:X}");
            }
            let definition = read_u64(&bytes, at + 8).unwrap() as usize;
            for field in [0, 4, 8] {
                let mut changed = bytes.clone();
                changed[definition + field] ^= 1;
                assert!(
                    layout(&changed, TAG).is_err(),
                    "definition {definition:X}+{field:X}"
                );
            }
            let mut changed = bytes.clone();
            write_u64(&mut changed, at + 8, u64::MAX).unwrap();
            assert!(layout(&changed, TAG).is_err());
        }
    }

    #[test]
    fn akimbo_core_allows_relocated_input_trigger_reload_but_pins_magazine() {
        let mut bytes = fixture();
        for (at, _, class, _) in [COMPONENTS[1], COMPONENTS[2], COMPONENTS[5]] {
            let new = 0x7000 + at;
            bytes.resize(bytes.len().max(new + 32), 0);
            write_u64(&mut bytes, at + 8, new as u64).unwrap();
            write_u32(&mut bytes, new, TAG).unwrap();
            write_u32(&mut bytes, new + 4, class).unwrap();
            write_u64(&mut bytes, new + 8, at as u64).unwrap();
        }
        let len = bytes.len() as u64;
        write_u64(&mut bytes, 0, len).unwrap();
        assert!(layout(&bytes, TAG).is_ok());
        write_u64(&mut bytes, 0x2558, 0x7000).unwrap();
        write_u32(&mut bytes, 0x7000, TAG).unwrap();
        write_u32(&mut bytes, 0x7004, 0x80803A0F).unwrap();
        write_u64(&mut bytes, 0x7008, 0x2550).unwrap();
        assert!(layout(&bytes, TAG).is_err());
        assert_eq!(inspect(&bytes, TAG).unwrap().definitions[4], 0x7000);
    }

    #[test]
    fn akimbo_source_layout_validates_root_and_reload_type_pairs_without_advertising_support() {
        for root in ROOT_KINDS {
            for reload in RELOAD_KINDS {
                let mut bytes = fixture();
                write_u32(&mut bytes, 0x84, root.0).unwrap();
                write_u32(&mut bytes, 0x3464, root.1).unwrap();
                write_u32(&mut bytes, 0x2794, reload.0).unwrap();
                write_u32(&mut bytes, 0x63C4, reload.1).unwrap();
                let derived = inspect(&bytes, TAG).unwrap();
                assert_eq!(derived.root_kind, root);
                assert_eq!(derived.reload_kind, reload);
                assert_eq!(
                    layout(&bytes, TAG).is_ok(),
                    root == ROOT_KINDS[0] && reload == RELOAD_KINDS[0]
                );
                bytes[0x3464] ^= 1;
                assert!(inspect(&bytes, TAG).is_err());
            }
        }
        for at in [0x84, 0x2794] {
            let mut bytes = fixture();
            write_u32(&mut bytes, at, 0xDEADBEEF).unwrap();
            assert!(inspect(&bytes, TAG).is_err());
        }
        let mut bytes = fixture();
        let near_end = bytes.len() - 16;
        write_u64(&mut bytes, 0x2558, near_end as u64).unwrap();
        write_u32(&mut bytes, near_end, TAG).unwrap();
        write_u32(&mut bytes, near_end + 4, 0x80803A0F).unwrap();
        write_u64(&mut bytes, near_end + 8, 0x2550).unwrap();
        // Typed ownership alone is recorded here. The host validates every property range
        // needed by magazine/reload access before the layout can become an active binding.
        assert_eq!(inspect(&bytes, TAG).unwrap().definitions[4], near_end);
    }
}
