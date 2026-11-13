//! Bullets per Shot as a setting of the weapon itself. The Barrel reads how many bullets one pull
//! fires from its inputs 22 to 25, and the stat translator writes them. The translation scripts
//! name the same output `bullets_per_shot` where they publish it as a channel. Where every input
//! copies the same column of one translation, and that column holds a whole number of bullets at
//! every stat tier, the column is the weapon's bullets per shot and a private translator copy
//! can hold another at every tier. Most types hold one number there, as a fusion rifle's seven
//! bolts do. A pulse rifle's, sidearm's and rocket launcher's change with Rounds Per Minute, so
//! the weapon's own value picks the tier, and a set value holds at every one. The other inputs
//! that read the column, such as the damage a fusion rifle splits over its bolts and the
//! ammunition a burst spends, follow it as they do in stock. A column that holds 0, as an
//! automatic weapon's does, is not offered.
use super::rig::{
    TRANSLATOR_KEY_CLASS, TRANSLATOR_KEY_HASH, TRANSLATOR_KEY_SIZE, TRANSLATOR_TABLE_CLASS,
    TRANSLATOR_TABLE_SIZE, translator_array,
};
use crate::tag_payload::{array_at, read_u32};
use crate::{AuthoringResult, error::invalid, item::WeaponRuntimeResourcePatch};
use sundial::package_authoring::PackageManager;
use sundial::package_authoring::entity::{
    WEAPON_STAT_TRANSLATOR_COMPONENT_KEY, weapon_component_bindings,
};
use sundial::package_authoring::runtime::modifiers::{BARREL_BULLETS_PER_SHOT, BARREL_COMPONENT};
use tiger_pkg::TagHash;

/// A type's translations, each a stat converted at every tier: its rows descriptor at +0x10, the
/// Weapon Stats input it converts at +0x20, and the stat values its first and last tiers stand for
/// at +0x28 and +0x2C, with the tiers evenly between.
const TRANSLATION_CLASS: u32 = 0x8080_3979;
const TRANSLATION_SIZE: usize = 0x38;
const TRANSLATION_ROWS: usize = 0x10;
const TRANSLATION_STAT: usize = 0x20;
const TRANSLATION_RANGE: usize = 0x28;
/// Weapon Stats input 0, which the investment constants map to Rounds Per Minute, and that stat's
/// definition hash. Every stock column that changes follows it.
const ROUNDS_PER_MINUTE_INPUT: u32 = 0;
pub(crate) const ROUNDS_PER_MINUTE_HASH: u32 = 0xFF66_4809;
/// One tier's outputs: a count and a pointer to that many Float32 values.
const ROW_CLASS: u32 = 0x8080_9F2A;
const ROW_SIZE: usize = 0x10;
const OUTPUTS_CLASS: u32 = 0x8080_000F;
/// A routing entry: the component at +0, its input at +4, how it combines at +0xC and, for a
/// direct copy, the output it copies at +0x1C.
const ROUTE_CLASS: u32 = 0x8080_3981;
const ROUTE_SIZE: usize = 0x50;
const ROUTE_COMPONENT: usize = 0x0;
const ROUTE_INPUT: usize = 0x4;
const ROUTE_OPERATION: usize = 0xC;
const ROUTE_SOURCE: usize = 0x1C;
/// Copies one translation output as it is.
const DIRECT: u32 = 4;
/// Translation outputs are numbered from here, 0x20 for each translation and one for each column.
const OUTPUTS: u32 = 0x1E00;
const OUTPUTS_PER_TRANSLATION: u32 = 0x20;

/// The column a weapon's bullets per shot is kept in.
pub(crate) struct Burst {
    pub(crate) column: Column,
    /// Each tier's value, as an offset into the translator owner.
    cells: Vec<usize>,
    /// The translator resource the patches are relative to.
    resource: usize,
}

/// The bullets per shot a translator gives at each stat tier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Column {
    /// Each tier's bullets, lowest stat first.
    tiers: Vec<u16>,
    /// The stat the tiers follow, by definition hash, where they differ.
    pub(crate) stat: Option<u32>,
    /// The stat values the first and last tiers stand for.
    range: [i32; 2],
    interpolate: bool,
    offset: i32,
}

impl Column {
    /// The bullets at every tier, where they agree.
    pub(crate) fn fixed(&self) -> Option<u16> {
        let first = *self.tiers.first()?;
        self.tiers
            .iter()
            .all(|&bullets| bullets == first)
            .then_some(first)
    }

    /// The bullets at stat `value`, from the nearest tier. Values outside the range take its end.
    pub(crate) fn at(&self, value: i32) -> u16 {
        let [low, high] = self.range;
        let last = self.tiers.len().saturating_sub(1);
        let tier = if high > low && last > 0 {
            let position = f64::from(value.clamp(low, high) - low) / f64::from(high - low);
            (position * last as f64).round() as usize
        } else {
            0
        };
        self.tiers.get(tier.min(last)).copied().unwrap_or(1)
    }

    /// Runtime-compatible sampling for admission checks. Unlike the UI's nearest-tier
    /// reading, a fractional interpolated bullet count is not rounded into a supported shot.
    pub(crate) fn selected(&self, value: i32) -> Option<u16> {
        if let Some(value) = self.fixed() {
            return Some(value);
        }
        if self.offset != 0 {
            return None;
        }
        let [low, high] = self.range;
        let last = self.tiers.len().checked_sub(1)?;
        if high <= low {
            return None;
        }
        let value = value.clamp(low, high);
        let position = (value - low) as f32 * last as f32 / (high - low) as f32;
        let lower = (position as usize).min(last);
        let mut sampled = f32::from(self.tiers[lower]);
        if self.interpolate {
            let next = f32::from(self.tiers[(lower + 1).min(last)]);
            sampled += (next - sampled) * (position - position.floor());
        }
        (sampled.is_finite() && sampled.fract() == 0.0).then_some(sampled as u16)
    }
}

/// The bullets per shot `entity` fires, converted by the table for translation `group`, when its
/// translator keeps them in one column. None for a weapon whose burst comes from elsewhere, holds
/// 0, or follows a stat other than Rounds Per Minute.
pub(crate) fn read(
    manager: &PackageManager,
    entity: &[u8],
    group: u32,
) -> AuthoringResult<Option<Burst>> {
    let bindings =
        weapon_component_bindings(entity, WEAPON_STAT_TRANSLATOR_COMPONENT_KEY).map_err(invalid)?;
    let [binding] = bindings.as_slice() else {
        return Ok(None);
    };
    let owner = manager
        .read_tag(TagHash(binding.owner_tag))
        .map_err(|error| invalid(error.to_string()))?;
    let resource = usize::try_from(binding.resource_offset)
        .map_err(|_| invalid("Stat translator offset overflow"))?;
    Ok(column(&owner, group)?.map(|(column, cells, _)| Burst {
        column,
        cells,
        resource,
    }))
}

/// The translation output every Barrel bullet input copies in translator `owner`'s table for
/// `group`, which a Bullets per Shot edit holds at every tier. None where this setting is not
/// offered.
pub(crate) fn output(owner: &[u8], group: u32) -> AuthoringResult<Option<u32>> {
    Ok(column(owner, group)?.map(|(_, _, output)| output))
}

/// Patches that make the translator give the Barrel `bullets` bullets per shot, in the private
/// translator copy the build makes. None for a weapon without the column, which does not offer the
/// setting, so a value saved on another Runtime stays inert until one with the column returns.
pub(crate) fn patches(
    manager: &PackageManager,
    entity: &[u8],
    group: u32,
    bullets: u16,
) -> AuthoringResult<Vec<WeaponRuntimeResourcePatch>> {
    let Some(burst) = read(manager, entity, group)? else {
        return Ok(Vec::new());
    };
    let bytes = f32::from(bullets).to_le_bytes().to_vec();
    burst
        .cells
        .iter()
        .map(|&cell| {
            let offset = cell
                .checked_sub(burst.resource)
                .and_then(|offset| u32::try_from(offset).ok())
                .ok_or_else(|| invalid("Stat translator patch offset overflow"))?;
            Ok(WeaponRuntimeResourcePatch {
                binding_hash: WEAPON_STAT_TRANSLATOR_COMPONENT_KEY,
                resource_index: 0,
                offset,
                bytes: bytes.clone(),
                graph_values: Vec::new(),
                graph_removals: Vec::new(),
                graph_trajectories: None,
            })
        })
        .collect()
}

/// The bullets, the cells and the output number of the column every Barrel bullet input copies,
/// in the table for `group`, or the first table when the translator has none for it, as the
/// client falls back.
pub(crate) fn column(
    owner: &[u8],
    group: u32,
) -> AuthoringResult<Option<(Column, Vec<usize>, u32)>> {
    column_inner(owner, group, false)
}

/// Admission-only form includes the native zero convention and proves the complete direct
/// operand. The editor keeps its existing nonzero setting semantics through `column`.
pub(crate) fn firing_column(owner: &[u8], group: u32) -> AuthoringResult<Option<Column>> {
    Ok(column_inner(owner, group, true)?.map(|(column, _, _)| column))
}

fn pins_preserve_input(owner: &[u8], key: usize, input: u32) -> AuthoringResult<bool> {
    if crate::tag_payload::read_u64(owner, key)? == 0 {
        return Ok(true);
    }
    let (count, _, rows, class) = array_at(owner, key)?;
    if class != 0x808038C9
        || count > 128
        || rows
            .checked_add(count * 8)
            .is_none_or(|end| end > owner.len())
    {
        return Ok(false);
    }
    let mut seen = std::collections::BTreeSet::new();
    for at in (0..count).map(|index| rows + index * 8) {
        // Native pins address a signed WeaponStats input byte and store one float32 value.
        // Another input's constant cannot replace the selected translation's authored stat.
        let pin = owner[at];
        if pin > i8::MAX as u8
            || owner[at + 1..at + 4] != [0; 3]
            || !seen.insert(pin)
            || u32::from(pin) == input
            || !f32::from_bits(read_u32(owner, at + 4)?).is_finite()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn column_inner(
    owner: &[u8],
    group: u32,
    firing: bool,
) -> AuthoringResult<Option<(Column, Vec<usize>, u32)>> {
    let keys = translator_array(owner, TRANSLATOR_KEY_CLASS)?;
    let tables = translator_array(owner, TRANSLATOR_TABLE_CLASS)?;
    if keys.0 == 0 || keys.0 != tables.0 {
        return Ok(None);
    }
    let mut selected = None;
    for key in 0..keys.0 {
        if read_u32(
            owner,
            keys.1 + key * TRANSLATOR_KEY_SIZE + TRANSLATOR_KEY_HASH,
        )? == group
        {
            if selected.replace(key).is_some() {
                return Ok(None);
            }
        }
    }
    let index = selected.unwrap_or(0);
    let table = tables.1 + index * TRANSLATOR_TABLE_SIZE;
    let (translations, _, translation_rows, translation_class) = array_at(owner, table)?;
    let (routes, _, route_rows, route_class) = array_at(owner, table + 0x10)?;
    if translation_class != TRANSLATION_CLASS || route_class != ROUTE_CLASS {
        return Ok(None);
    }
    let mut source = None;
    for input in BARREL_BULLETS_PER_SHOT {
        let mut found = None;
        for route in (0..routes).map(|index| route_rows + index * ROUTE_SIZE) {
            if i64::from(read_u32(owner, route + ROUTE_COMPONENT)?) == BARREL_COMPONENT
                && i64::from(read_u32(owner, route + ROUTE_INPUT)?) == input
            {
                if found.is_some() {
                    return Ok(None);
                }
                found = Some(route);
            }
        }
        let Some(route) = found else {
            return Ok(None);
        };
        let own = read_u32(owner, route + ROUTE_SOURCE)?;
        if read_u32(owner, route + ROUTE_OPERATION)? != DIRECT
            || source.is_some_and(|s| s != own)
            || (firing
                && (read_u32(owner, route + 0x10)? != 0
                    || read_u32(owner, route + 0x20)? != f32::MAX.to_bits()))
        {
            return Ok(None);
        }
        source = Some(own);
    }
    let Some(source) = source else {
        return Ok(None);
    };
    let Some(output) = source.checked_sub(OUTPUTS) else {
        return Ok(None);
    };
    let translation = (output / OUTPUTS_PER_TRANSLATION) as usize;
    let column = (output % OUTPUTS_PER_TRANSLATION) as usize;
    if translation >= translations {
        return Ok(None);
    }
    let translation = translation_rows + translation * TRANSLATION_SIZE;
    let (tiers, _, tier_rows, tier_class) = array_at(owner, translation + TRANSLATION_ROWS)?;
    if tier_class != ROW_CLASS || tiers == 0 {
        return Ok(None);
    }
    let mut cells = Vec::with_capacity(tiers);
    let mut bullets = Vec::with_capacity(tiers);
    for tier in (0..tiers).map(|index| tier_rows + index * ROW_SIZE) {
        let (count, _, outputs, class) = array_at(owner, tier)?;
        if class != OUTPUTS_CLASS || column >= count {
            return Ok(None);
        }
        let cell = outputs + column * 4;
        let value = f32::from_bits(read_u32(owner, cell)?);
        if value.fract() != 0.0
            || !(if firing { 0.0 } else { 1.0 }
                ..=f32::from(crate::weapon::barrel::MAX_BULLETS_PER_SHOT))
                .contains(&value)
        {
            return Ok(None);
        }
        bullets.push(value as u16);
        cells.push(cell);
    }
    let bound = |at: usize| -> AuthoringResult<Option<i32>> {
        let value = f32::from_bits(read_u32(owner, translation + TRANSLATION_RANGE + at)?);
        Ok((value.fract() == 0.0 && value.abs() <= 1.0e6).then_some(value as i32))
    };
    let (Some(low), Some(high)) = (bound(0)?, bound(4)?) else {
        return Ok(None);
    };
    let mut column = Column {
        tiers: bullets,
        stat: None,
        range: [low, high],
        interpolate: owner.get(translation + 0x30).copied() == Some(1),
        offset: read_u32(owner, translation + 0x24)? as i32,
    };
    if column.fixed().is_none() {
        // Only a stat the weapon's own stats name can pick the tier the editor shows.
        if read_u32(owner, translation + TRANSLATION_STAT)? != ROUNDS_PER_MINUTE_INPUT {
            return Ok(None);
        }
        column.stat = Some(ROUNDS_PER_MINUTE_HASH);
        if firing {
            if owner.get(translation + 0x30).is_none_or(|value| *value > 1) {
                return Ok(None);
            }
            let key = keys.1 + index * TRANSLATOR_KEY_SIZE;
            if !pins_preserve_input(owner, key, read_u32(owner, translation + TRANSLATION_STAT)?)? {
                // A pin of this exact input needs its own sampling path. Do not silently
                // use the final item stat when the native translator substitutes a constant.
                return Ok(None);
            }
        }
    }
    Ok(Some((column, cells, source)))
}

#[cfg(test)]
mod akimbo_tests {
    use super::*;
    use crate::tag_payload::{append_native_array, write_u32};

    fn translator(groups: &[u32]) -> Vec<u8> {
        let mut owner = vec![0; 32];
        let mut keys = vec![0; groups.len() * TRANSLATOR_KEY_SIZE];
        for (index, group) in groups.iter().enumerate() {
            write_u32(
                &mut keys,
                index * TRANSLATOR_KEY_SIZE + TRANSLATOR_KEY_HASH,
                *group,
            )
            .unwrap();
        }
        append_native_array(&mut owner, 0, TRANSLATOR_KEY_CLASS, groups.len(), &keys).unwrap();
        append_native_array(
            &mut owner,
            16,
            TRANSLATOR_TABLE_CLASS,
            groups.len(),
            &vec![0; groups.len() * TRANSLATOR_TABLE_SIZE],
        )
        .unwrap();
        if groups.is_empty() {
            return owner;
        }
        let (_, _, tables, _) = array_at(&owner, 16).unwrap();
        for index in 0..groups.len() {
            let table = tables + index * TRANSLATOR_TABLE_SIZE;
            append_native_array(
                &mut owner,
                table,
                TRANSLATION_CLASS,
                1,
                &[0; TRANSLATION_SIZE],
            )
            .unwrap();
            let (_, _, translation, _) = array_at(&owner, table).unwrap();
            write_u32(
                &mut owner,
                translation + TRANSLATION_RANGE + 4,
                100f32.to_bits(),
            )
            .unwrap();
            append_native_array(
                &mut owner,
                translation + TRANSLATION_ROWS,
                ROW_CLASS,
                1,
                &[0; ROW_SIZE],
            )
            .unwrap();
            let (_, _, tier, _) = array_at(&owner, translation + TRANSLATION_ROWS).unwrap();
            append_native_array(
                &mut owner,
                tier,
                OUTPUTS_CLASS,
                1,
                &(2.0 + index as f32).to_le_bytes(),
            )
            .unwrap();
            let mut routes = vec![0; BARREL_BULLETS_PER_SHOT.len() * ROUTE_SIZE];
            for (lane, input) in BARREL_BULLETS_PER_SHOT.into_iter().enumerate() {
                let at = lane * ROUTE_SIZE;
                for (field, value) in [
                    (ROUTE_COMPONENT, BARREL_COMPONENT as u32),
                    (ROUTE_INPUT, input as u32),
                    (ROUTE_OPERATION, DIRECT),
                    (ROUTE_SOURCE, OUTPUTS),
                    (0x20, f32::MAX.to_bits()),
                ] {
                    write_u32(&mut routes, at + field, value).unwrap();
                }
            }
            append_native_array(
                &mut owner,
                table + 16,
                ROUTE_CLASS,
                BARREL_BULLETS_PER_SHOT.len(),
                &routes,
            )
            .unwrap();
        }
        owner
    }

    #[test]
    fn akimbo_firing_columns_require_nonempty_unambiguous_groups_and_preserve_native_fallback() {
        assert!(firing_column(&translator(&[]), 2).is_err());
        let valid = translator(&[1, 2]);
        assert_eq!(firing_column(&valid, 2).unwrap().unwrap().fixed(), Some(3));
        assert_eq!(firing_column(&valid, 99).unwrap().unwrap().fixed(), Some(2));
        assert!(firing_column(&translator(&[2, 2]), 2).unwrap().is_none());
        let mut mismatched = translator(&[1, 2]);
        let (_, header, _, _) = array_at(&mismatched, 16).unwrap();
        crate::tag_payload::set_array_count(&mut mismatched, 16, header, 1).unwrap();
        assert!(firing_column(&mismatched, 2).unwrap().is_none());
    }

    #[test]
    fn akimbo_variable_firing_column_accepts_only_typed_irrelevant_input_pins() {
        let mut base = translator(&[1]);
        let (_, tables) = translator_array(&base, TRANSLATOR_TABLE_CLASS).unwrap();
        let (_, _, translation, _) = array_at(&base, tables).unwrap();
        append_native_array(
            &mut base,
            translation + TRANSLATION_ROWS,
            ROW_CLASS,
            2,
            &[0; 32],
        )
        .unwrap();
        let (_, _, tiers, _) = array_at(&base, translation + TRANSLATION_ROWS).unwrap();
        for (index, value) in [2f32, 3.0].into_iter().enumerate() {
            append_native_array(
                &mut base,
                tiers + index * 16,
                OUTPUTS_CLASS,
                1,
                &value.to_le_bytes(),
            )
            .unwrap();
        }
        assert!(firing_column(&base, 1).unwrap().is_some());
        let (_, key) = translator_array(&base, TRANSLATOR_KEY_CLASS).unwrap();
        let with_pins = |class, pins: &[[u32; 2]]| {
            let mut owner = base.clone();
            let bytes = pins
                .iter()
                .flatten()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            append_native_array(&mut owner, key, class, pins.len(), &bytes).unwrap();
            owner
        };
        let unrelated = with_pins(0x808038C9, &[[16, 50f32.to_bits()]]);
        assert_eq!(
            firing_column(&unrelated, 1).unwrap().unwrap().selected(100),
            Some(3)
        );
        for pins in [
            vec![[0, 50f32.to_bits()]],
            vec![[16, 50f32.to_bits()], [16, 1f32.to_bits()]],
            vec![[16, f32::NAN.to_bits()]],
            vec![[0x110, 50f32.to_bits()]],
            vec![[255, 50f32.to_bits()]],
        ] {
            assert!(
                firing_column(&with_pins(0x808038C9, &pins), 1)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            firing_column(&with_pins(ROW_CLASS, &[[16, 50f32.to_bits()]]), 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn akimbo_admission_samples_runtime_tiers_without_nearest_rounding() {
        let mut column = Column {
            tiers: vec![1, 1, 1, 2, 1, 3, 1, 1, 1, 1, 1],
            stat: Some(ROUNDS_PER_MINUTE_HASH),
            range: [0, 100],
            interpolate: true,
            offset: 0,
        };
        assert_eq!(column.selected(40), Some(1));
        assert_eq!(column.selected(50), Some(3));
        assert_eq!(column.selected(25), None);
        assert_eq!(column.selected(45), Some(2));
        assert_eq!(column.selected(-1), Some(1));
        assert_eq!(column.selected(101), Some(1));
        column.interpolate = false;
        assert_eq!(column.selected(29), Some(1));
        column.offset = 1;
        assert_eq!(column.selected(40), None);
    }
}
