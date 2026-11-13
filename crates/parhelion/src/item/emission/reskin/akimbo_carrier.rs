//! A runtime identity must be drawn for every native choice of sight or other part.
use super::*;

/// Alternatives are not identities: a sorted key may name an unequipped sight.
/// Select one paired owner from a fixed assignment or a region with one choice.
/// `paired` contains only parts whose model emitter actually duplicated geometry.
pub(super) fn select(
    metadata: &[u8],
    row: usize,
    paired: &BTreeMap<u32, String>,
) -> AuthoringResult<String> {
    let mut fixed = BTreeSet::from([read_u32(metadata, row + 8)?, read_u32(metadata, row + 12)?]);
    let (count, _, rows, _) = array(metadata, row + 16)?;
    for index in 0..count {
        let resource = relative_target(metadata, rows + index * 8)?;
        let (choices, _, assignments, _) = array(metadata, resource + 8)?;
        if choices == 1 {
            fixed.insert(read_u32(metadata, assignments)?);
        }
    }
    fixed.into_iter()
        .filter(|key| !matches!(*key, 0 | u32::MAX | 0x811C9DC5))
        .find_map(|key| paired.get(&key).cloned())
        .ok_or_else(|| invalid("Akimbo appearance has no guaranteed singleton paired model owner; optional parts cannot identify the equipped weapon"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::{append_native_array, write_i64, write_u64};

    fn metadata(slots: &[&[u32]]) -> Vec<u8> {
        let mut bytes = vec![0; 0x80];
        let mut resources = Vec::new();
        for (index, keys) in slots.iter().enumerate() {
            let resource = bytes.len();
            bytes.resize(resource + 24, 0);
            write_u64(&mut bytes, resource, index as u64).unwrap();
            let values = keys
                .iter()
                .flat_map(|key| key.to_le_bytes())
                .collect::<Vec<_>>();
            append_native_array(&mut bytes, resource + 8, 0x80805DFF, keys.len(), &values).unwrap();
            resources.push(resource);
        }
        append_native_array(
            &mut bytes,
            0x50,
            0x80805DFE,
            slots.len(),
            &vec![0; slots.len() * 8],
        )
        .unwrap();
        let (_, _, rows, _) = array(&bytes, 0x50).unwrap();
        for (index, resource) in resources.into_iter().enumerate() {
            let at = rows + index * 8;
            write_i64(&mut bytes, at, resource as i64 - at as i64).unwrap();
        }
        bytes
    }

    #[test]
    fn akimbo_carrier_skips_sorted_inactive_sight_and_retained_reticle() {
        // The lowest source key is an optional sight, as in native Antiope's
        // slot15 alternatives. A retained reticle is fixed but not paired.
        let metadata = metadata(&[&[100], &[2], &[1, 3, 4, 5, 6, 7, 8]]);
        let paired = BTreeMap::from([
            (1, "optional-sight-owner".into()),
            (100, "body-owner".into()),
        ]);
        assert_eq!(select(&metadata, 0x40, &paired).unwrap(), "body-owner");
        assert!(
            select(
                &metadata,
                0x40,
                &BTreeMap::from([(1, "optional-sight-owner".into())])
            )
            .is_err()
        );
    }

    #[test]
    fn akimbo_carrier_never_falls_back_to_an_optional_model() {
        let mut metadata = metadata(&[&[1, 2], &[3, 4]]);
        let paired = BTreeMap::from([(1, "optional-owner".into()), (50, "fixed-owner".into())]);
        assert!(select(&metadata, 0x40, &paired).is_err());
        write_u32(&mut metadata, 0x48, 50).unwrap();
        assert_eq!(select(&metadata, 0x40, &paired).unwrap(), "fixed-owner");
    }
}
