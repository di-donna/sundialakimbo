//! Resolve a variable firing tier from the authored definition and the live stat hash table.
use super::*;
use crate::tag_payload::{read_i32, read_u8};

const STAT_DEFINITIONS_SLOT: usize = 95;
const STAT_DEFINITION_ROW_CLASS: u32 = 0x80807D09;
const STAT_DEFINITION_ROW_SIZE: usize = 32;
const RPM_INPUT_DEFINITION: usize = 14;

fn stat_index(table: &[u8], hash: u32) -> AuthoringResult<(usize, usize)> {
    let (count, _, rows, class) = array(table, 8)?;
    if class != STAT_DEFINITION_ROW_CLASS
        || count == 0
        || count > 256
        || rows
            .checked_add(count * STAT_DEFINITION_ROW_SIZE)
            .is_none_or(|end| end > table.len())
    {
        return Err(invalid("Akimbo stat-definition table layout differs"));
    }
    let matches = (0..count)
        .filter(|index| read_u32(table, rows + index * STAT_DEFINITION_ROW_SIZE).ok() == Some(hash))
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return Err(invalid("Akimbo stat hash is missing or ambiguous"));
    };
    Ok((*index, count))
}

fn base_stat(definition: &[u8], index: usize, definitions: usize) -> AuthoringResult<i32> {
    let resource = relative_target(definition, ITEM_INVESTMENT_STAT_POINTER_OFFSET)?;
    if resource < 4 || read_u32(definition, resource - 4)? != ITEM_INVESTMENT_STAT_RESOURCE_CLASS {
        return Err(invalid(
            "Akimbo authored definition has no typed investment-stat resource",
        ));
    }
    let (count, _, rows, class) = array(definition, resource)?;
    if class != ITEM_INVESTMENT_STAT_ROW_CLASS
        || count == 0
        || count > 256
        || rows
            .checked_add(count * ITEM_INVESTMENT_STAT_ROW_SIZE)
            .is_none_or(|end| end > definition.len())
    {
        return Err(invalid(
            "Akimbo authored stat rows have an unsupported layout",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut value = None;
    for at in (0..count).map(|row| rows + row * ITEM_INVESTMENT_STAT_ROW_SIZE) {
        let own = usize::from(read_u8(definition, at)?);
        if own >= definitions || read_u8(definition, at + 1)? != 0 || !seen.insert(own) {
            return Err(invalid(
                "Akimbo authored stat index is reserved or repeated",
            ));
        }
        if own == index {
            // Both amount and activation programs can change the chosen runtime tier.
            for offset in [8, 24] {
                let descriptor = at + offset;
                crate::item::validate_item_numeric_program(definition, descriptor)?;
                if read_u64(definition, descriptor)? != 0 {
                    return Err(invalid(
                        "Akimbo firing stat uses an amount or activation program",
                    ));
                }
            }
            value = Some(read_i32(definition, at + 4)?);
        }
    }
    value.ok_or_else(|| invalid("Akimbo authored definition omits the firing stat"))
}

pub(super) fn selected_count(
    manager: &PackageManager,
    emission: &PackageEmission,
    item: u32,
    entity: &[u8],
    group: u32,
    read: impl Fn(u32) -> AuthoringResult<Vec<u8>>,
) -> AuthoringResult<u16> {
    let bindings = weapon_component_bindings(
        entity,
        sundial::package_authoring::entity::WEAPON_STAT_TRANSLATOR_COMPONENT_KEY,
    )
    .map_err(invalid)?;
    let [binding] = bindings.as_slice() else {
        return Err(invalid("Akimbo needs one final stat translator"));
    };
    if binding.resource_count != 1 {
        return Err(invalid(
            "Akimbo final stat translator has multiple resources",
        ));
    }
    let column = crate::weapon::burst::firing_column(&read(binding.owner_tag)?, group)?
        .ok_or_else(|| invalid("Akimbo bullet inputs do not share a proven firing column"))?;
    let selected = if let Some(fixed) = column.fixed() {
        Some(fixed)
    } else {
        if column.stat != Some(crate::weapon::burst::ROUNDS_PER_MINUTE_HASH) {
            return Err(invalid("Akimbo firing column follows an unsupported stat"));
        }
        let globals =
            resolve_live_named_tag(manager, "investment_globals", None).map_err(invalid)?;
        let root = globals_child_tag(&read(globals.0)?, 0)?;
        if manager
            .get_entry(root)
            .is_none_or(|entry| entry.reference != INVESTMENT_ROOT_CLASS)
        {
            return Err(invalid("Akimbo stat lookup has no typed investment root"));
        }
        let definitions = root_child_tag(&read(root.0)?, STAT_DEFINITIONS_SLOT)?;
        let (index, count) = stat_index(
            &read(definitions.0)?,
            crate::weapon::burst::ROUNDS_PER_MINUTE_HASH,
        )?;
        if index != RPM_INPUT_DEFINITION {
            return Err(invalid("Akimbo native RPM input/stat hash mapping changed"));
        }
        let ordinal = super::super::super::definition_ordinal(emission, item)?;
        let definition = &emission.host_new_tags[ordinal].payload;
        if read_u32(definition, ITEM_DEFINITION_HASH_OFFSET)? != item {
            return Err(invalid(
                "Akimbo final item definition hash differs from its assignment",
            ));
        }
        column.selected(base_stat(definition, index, count)?)
    };
    // This proves the serialized base tier. The native adapter still checks all four
    // live bullet inputs before issuing a shot, including later perk/stat transitions.
    selected.filter(|count| *count <= 64).ok_or_else(|| {
        invalid("Akimbo firing count must resolve exactly to an integer from 0 through 64")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::{append_native_array, write_relative_pointer};

    fn definition() -> Vec<u8> {
        let mut bytes = vec![0; 0x140];
        write_relative_pointer(&mut bytes, ITEM_INVESTMENT_STAT_POINTER_OFFSET, 0x100).unwrap();
        write_u32(&mut bytes, 0xFC, ITEM_INVESTMENT_STAT_RESOURCE_CLASS).unwrap();
        let mut rows = vec![0; 2 * ITEM_INVESTMENT_STAT_ROW_SIZE];
        rows[0] = 14;
        write_u32(&mut rows, 4, 40).unwrap();
        rows[ITEM_INVESTMENT_STAT_ROW_SIZE] = 16;
        append_native_array(&mut bytes, 0x100, ITEM_INVESTMENT_STAT_ROW_CLASS, 2, &rows).unwrap();
        bytes
    }

    #[test]
    fn akimbo_firing_stat_reads_final_payload_and_rejects_dynamic_or_ambiguous_values() {
        let mut bytes = definition();
        assert_eq!(base_stat(&bytes, 14, 32).unwrap(), 40);
        let rows = array(&bytes, 0x100).unwrap().2;
        write_u32(&mut bytes, rows + 4, 50).unwrap();
        assert_eq!(base_stat(&bytes, 14, 32).unwrap(), 50);
        assert!(base_stat(&bytes, 15, 32).is_err());
        assert!(base_stat(&bytes, 14, 16).is_err());
        for offset in [8, 24] {
            let mut changed = bytes.clone();
            write_u32(&mut changed, rows + offset, 1).unwrap();
            assert!(base_stat(&changed, 14, 32).is_err());
        }
        bytes[rows + ITEM_INVESTMENT_STAT_ROW_SIZE] = 14;
        assert!(base_stat(&bytes, 14, 32).is_err());
        bytes[rows + ITEM_INVESTMENT_STAT_ROW_SIZE] = 16;
        bytes[rows + 1] = 1;
        assert!(base_stat(&bytes, 14, 32).is_err());
    }

    #[test]
    fn akimbo_firing_stat_identity_comes_from_the_typed_table() {
        let mut table = vec![0; 24];
        let mut rows = vec![0; 16 * STAT_DEFINITION_ROW_SIZE];
        write_u32(
            &mut rows,
            14 * STAT_DEFINITION_ROW_SIZE,
            crate::weapon::burst::ROUNDS_PER_MINUTE_HASH,
        )
        .unwrap();
        append_native_array(&mut table, 8, STAT_DEFINITION_ROW_CLASS, 16, &rows).unwrap();
        assert_eq!(
            stat_index(&table, crate::weapon::burst::ROUNDS_PER_MINUTE_HASH).unwrap(),
            (14, 16)
        );
        let rows = array(&table, 8).unwrap().2;
        write_u32(
            &mut table,
            rows + 15 * STAT_DEFINITION_ROW_SIZE,
            crate::weapon::burst::ROUNDS_PER_MINUTE_HASH,
        )
        .unwrap();
        assert!(stat_index(&table, crate::weapon::burst::ROUNDS_PER_MINUTE_HASH).is_err());
    }
}
