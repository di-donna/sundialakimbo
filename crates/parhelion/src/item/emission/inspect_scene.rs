//! Private inspect scenes for weapons whose equipment skeleton has grown.
//!
//! The inspect screen shows an item in the crosshair-table scene its strings name by bucket and
//! first type key. The row's style row begins with the key of the scene object that shows the
//! weapon. Each object names an anchor piece and an animated piece. The weapon object's anchor
//! binds the equipment skeleton and markers of one weapon family, and its controls either there or
//! on the animated piece, as controls of the scene's own with the same bones and slots. Other
//! objects carry crosshair elements or a character body. The screen draws the gear model on the
//! weapon piece, and creates no display object when the model's geometry palette has more bones
//! than that piece's skeleton. An import that appends bones to its equipment skeleton has such a
//! palette while every stock piece keeps a stock skeleton, so its inspect screen stays empty.
//!
//! Such a weapon gets a private copy of a stock scene whose weapon object anchors a piece of its
//! own family: the item's scene when it has one, else the first such scene of the item's bucket,
//! then of any bucket. The weapon object's pieces bind copies of the converted equipment rigs,
//! moved as the runtime entity's are. Scene controls are extended as the converted controls
//! are, so every bone, appended or renumbered, takes its rest pose from them. Copies of the object
//! list, scene root and pair name the pieces. The pair loads the scene through a companion cloned
//! from the native pair's. A row of the item's own names the pair under the item's bucket and a
//! private key, with the item's style row naming the copied weapon object, and only the strings'
//! first type key moves to it. The hip-fire crosshair follows the runtime content's type key, as
//! do the first-person parameters that key names, so neither changes.
use super::*;
use crate::weapon::crosshair::{Row, TABLE, Table};
use parhelion_import::tiger::{animation::rig, payload::Payload};
use sundial::package_authoring::entity::{
    retarget_weapon_component_owner, retarget_weapon_component_owner_payload,
};

/// An entity's component rows, each led by its owner tag.
const COMPONENTS: usize = 0x10;
const COMPONENT_ROW: u32 = 0x8080_9C04;
const COMPONENT_ROW_SIZE: usize = 12;
const CONTROLS: u32 = 0x8080_8F8F;
const OBJECT_LIST: u32 = 0x8080_6647;
/// The object list's objects, from its definition, with the anchor piece at +0x30 and the
/// animated piece at +0x38.
const OBJECTS: usize = 0x218;
const OBJECT: u32 = 0x8080_664F;
const OBJECT_SIZE: usize = 0x40;
/// Words standing in for the private rig copies until the linker assigns their tags. A payload
/// that already holds one cannot take them.
const STAND_INS: std::ops::Range<u32> = 0x8FFF_FF00..0x8FFF_FF10;
const PAIR: &str = "inspect-pair";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Skeleton,
    Controls,
    #[cfg_attr(not(feature = "d2-model-importer"), allow(dead_code))]
    Markers,
}

/// A converted equipment rig of the weapon: the stock owner and its payload, the converted
/// payload and how the conversion moved the owner's interior.
pub(super) struct Rig {
    pub(super) native: u32,
    pub(super) kind: Kind,
    pub(super) original: Payload,
    pub(super) converted: Vec<u8>,
    pub(super) relocation: Option<(usize, usize)>,
}

fn read(manager: &PackageManager, tag: u32) -> AuthoringResult<Vec<u8>> {
    manager
        .read_tag(TagHash(tag))
        .map_err(|e| invalid(format!("Inspect scene asset {tag:08X}: {e}")))
}

fn checked<T>(result: anyhow::Result<T>, what: &str) -> AuthoringResult<T> {
    result.map_err(|e| invalid(format!("Inspect scene {what}: {e:#}")))
}

/// The owners an entity binds, in component order.
fn components(entity: &[u8]) -> AuthoringResult<Vec<u32>> {
    let entity = Payload(entity.to_vec());
    checked(
        entity
            .array(COMPONENTS, COMPONENT_ROW_SIZE, Some(COMPONENT_ROW))
            .and_then(|rows| {
                rows.into_iter()
                    .map(|row| entity.u32(row))
                    .collect::<anyhow::Result<Vec<_>>>()
            }),
        "components",
    )
}

/// The definition class of an owner payload.
fn class(owner: &Payload) -> anyhow::Result<u32> {
    let base = owner.pointer(24)?;
    anyhow::ensure!(base >= 4, "owner has no definition");
    owner.u32(base - 4)
}

/// A stock crosshair scene: its pair, root entity and object list, each object's key, anchor and
/// animated piece, and the pieces present among them.
struct Scene {
    pair: u32,
    root: u32,
    list: u32,
    objects: Vec<(u32, u32, u32)>,
    pieces: Vec<u32>,
}

fn scene(manager: &PackageManager, pair: u32) -> AuthoringResult<Scene> {
    let record = read(manager, pair)?;
    if record.len() != 8 {
        return Err(invalid("Inspect scene pair layout differs"));
    }
    let root = read_u32(&record, 0)?;
    let mut lists = Vec::new();
    for owner in components(&read(manager, root)?)? {
        let payload = Payload(read(manager, owner)?);
        if checked(class(&payload), "component")? == OBJECT_LIST {
            lists.push((owner, payload));
        }
    }
    let [(owner, list)] = lists.as_slice() else {
        return Err(invalid("Inspect scene has no single object list"));
    };
    let base = checked(list.pointer(24), "object list")?;
    let mut objects = Vec::new();
    let mut pieces = Vec::new();
    for object in checked(
        list.array(base + OBJECTS, OBJECT_SIZE, Some(OBJECT)),
        "objects",
    )? {
        let field = |at| checked(list.u32(object + at), "object");
        let (key, anchor, animated) = (field(0x28)?, field(0x30)?, field(0x38)?);
        objects.push((key, anchor, animated));
        for piece in [anchor, animated] {
            if manager.get_entry(TagHash(piece)).is_some() && !pieces.contains(&piece) {
                pieces.push(piece);
            }
        }
    }
    Ok(Scene {
        pair,
        root,
        list: *owner,
        objects,
        pieces,
    })
}

/// The weapon object a style row names, as its anchor and animated piece, when its anchor binds
/// `skeleton`.
fn weapon_object(
    manager: &PackageManager,
    scene: &Scene,
    shown: u32,
    skeleton: u32,
) -> AuthoringResult<Option<[u32; 2]>> {
    let Some(&(_, anchor, animated)) = scene.objects.iter().find(|object| object.0 == shown) else {
        return Ok(None);
    };
    if !scene.pieces.contains(&anchor) || !components(&read(manager, anchor)?)?.contains(&skeleton)
    {
        return Ok(None);
    }
    Ok(Some([anchor, animated]))
}

fn holds_stand_in(data: &[u8]) -> bool {
    data.chunks_exact(4)
        .any(|word| STAND_INS.contains(&u32::from_le_bytes(word.try_into().unwrap())))
}

/// A private copy before linking: the words naming another copy, a stock tag it replaces or a
/// stand-in, become the linker's placeholders.
fn node(
    symbol: &str,
    template: u32,
    mut payload: Vec<u8>,
    names: &BTreeMap<u32, String>,
) -> AuthoringResult<linking::Node> {
    let mut patches = Vec::new();
    for at in (0..payload.len().saturating_sub(3)).step_by(4) {
        if let Some(target) = names.get(&read_u32(&payload, at)?) {
            write_u32(&mut payload, at, u32::MAX)?;
            patches.push((at, target.clone()));
        }
    }
    let mut node = linking::Node::new(symbol, template, payload);
    node.patches = patches;
    Ok(node)
}

/// Private copies of scene pieces and the owners they bind, before linking, with the stock tags
/// and stand-ins whose words name them.
struct Copies {
    drafts: Vec<(String, u32, Vec<u8>)>,
    names: BTreeMap<u32, String>,
}

/// Copies of the weapon object's pieces and of any other piece binding a converted rig. Each binds
/// the converted rigs and extended copies of its scene controls in place of the stock owners. A
/// piece that binds neither stays shared.
fn copies(
    manager: &PackageManager,
    scene: &Scene,
    object: [u32; 2],
    rigs: &[Rig],
    renumbering: &rig::Renumbering,
) -> AuthoringResult<Copies> {
    let natives = rigs.iter().map(|rig| rig.native).collect::<Vec<_>>();
    let mut stand_ins = STAND_INS;
    let mut pieces = 0;
    let mut copies = Copies {
        drafts: Vec::new(),
        names: BTreeMap::new(),
    };
    for &piece in &scene.pieces {
        let mut entity = read(manager, piece)?;
        let bound = components(&entity)?;
        if !object.contains(&piece) && !bound.iter().any(|owner| natives.contains(owner)) {
            continue;
        }
        if holds_stand_in(&entity) {
            return Err(invalid("Inspect scene piece holds a private rig stand-in"));
        }
        let mut owners = Vec::new();
        for converted in rigs.iter().filter(|rig| bound.contains(&rig.native)) {
            if holds_stand_in(&converted.converted) {
                return Err(invalid("Converted rig holds a private rig stand-in"));
            }
            // The same order of moves as the runtime entity's: rebase, owner, entity, typed
            // handles.
            if let Some((cut, delta)) = converted.relocation {
                checked(
                    rig::rebase_entity(
                        &mut entity,
                        &converted.original,
                        converted.native,
                        cut,
                        delta,
                    ),
                    "rig relocation",
                )?;
            }
            owners.push((converted.native, converted.converted.clone()));
        }
        for &owner in bound.iter().filter(|owner| !natives.contains(owner)) {
            let stock = Payload(read(manager, owner)?);
            if checked(class(&stock), "component")? != CONTROLS {
                continue;
            }
            let mut converted = rigs.iter().filter(|rig| rig.kind == Kind::Controls);
            let (Some(controls), None) = (converted.next(), converted.next()) else {
                return Err(invalid(
                    "Inspect scene controls have no single converted counterpart",
                ));
            };
            let extended = checked(
                rig::extend_controls_like(
                    &stock,
                    &controls.original,
                    &Payload(controls.converted.clone()),
                    renumbering,
                ),
                "controls",
            )?;
            if holds_stand_in(&extended.0) {
                return Err(invalid(
                    "Inspect scene controls hold a private rig stand-in",
                ));
            }
            owners.push((owner, extended.0));
        }
        if owners.is_empty() {
            continue;
        }
        let name = format!("inspect-piece-{pieces}");
        pieces += 1;
        copies.names.insert(piece, name.clone());
        for (native, mut owner) in owners {
            let stand_in = stand_ins
                .next()
                .ok_or_else(|| invalid("Inspect scene binds too many converted rigs"))?;
            retarget_weapon_component_owner_payload(&mut owner, &entity, native, stand_in)
                .map_err(|e| invalid(format!("Inspect scene rig: {e}")))?;
            retarget_weapon_component_owner(&mut entity, native, stand_in)
                .map_err(|e| invalid(format!("Inspect scene piece: {e}")))?;
            checked(
                rig::retarget_rebased_references(&mut entity, owner.len(), native, stand_in),
                "rig references",
            )?;
            let symbol = format!("{name}-rig-{}", copies.drafts.len());
            copies.names.insert(stand_in, symbol.clone());
            copies.drafts.push((symbol, native, owner));
        }
        copies.drafts.push((name, piece, entity));
    }
    Ok(copies)
}

/// The private scene's assets: the piece copies, copies of the object list, root and pair, and
/// the pair's loading companion.
fn nodes(
    (directory, manager): (&Path, &PackageManager),
    source: &Scene,
    copies: Copies,
    companions: &mut linking::Companions,
) -> AuthoringResult<Vec<linking::Node>> {
    let Copies {
        mut drafts,
        mut names,
    } = copies;
    names.extend([
        (source.pair, PAIR.to_owned()),
        (source.root, "inspect-root".to_owned()),
        (source.list, "inspect-objects".to_owned()),
    ]);
    for (symbol, tag) in [
        ("inspect-objects", source.list),
        ("inspect-root", source.root),
        (PAIR, source.pair),
    ] {
        let payload = read(manager, tag)?;
        if holds_stand_in(&payload) {
            return Err(invalid("Inspect scene asset holds a private rig stand-in"));
        }
        drafts.push((symbol.to_owned(), tag, payload));
    }
    let mut nodes = drafts
        .into_iter()
        .map(|(symbol, template, payload)| node(&symbol, template, payload, &names))
        .collect::<AuthoringResult<Vec<_>>>()?;
    let template = linking::native_companion(directory, manager, source.pair, companions)?.0;
    let mut companion = linking::Node::new(format!("{PAIR}-companion"), template.0, Vec::new());
    companion.companion = Some(linking::Companion::new(PAIR, source.pair));
    nodes.push(companion);
    Ok(nodes)
}

/// FNV-1 over the bytes of `name`.
fn fnv1(name: &str) -> u32 {
    name.bytes().fold(0x811C_9DC5u32, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ u32::from(byte)
    })
}

/// Imports historically permit an unchanged inspect scene; generated rigs require one that
/// can display the expanded palette before the build can succeed.
fn unavailable(required: bool, message: &str) -> AuthoringResult<Option<ReplacementSpec>> {
    if required {
        Err(invalid(message))
    } else {
        Ok(None)
    }
}

fn mapping(
    skeleton: &Rig,
    supplied: Option<&rig::Renumbering>,
) -> AuthoringResult<rig::Renumbering> {
    let inferred = checked(
        rig::renumbering(&skeleton.original, &Payload(skeleton.converted.clone())),
        "skeleton renumbering",
    )?;
    if supplied.is_some_and(|mapping| {
        mapping.map != inferred.map || mapping.first_appended != inferred.first_appended
    }) {
        return Err(invalid(
            "Inspect scene renumbering differs from its converted skeleton",
        ));
    }
    Ok(inferred)
}

/// Give the item a private inspect scene when its converted equipment skeleton has more bones
/// than the stock one. `table` is the crosshair table as earlier steps left it. Returns that table
/// with the item's row added. A supplied bone mapping must match the converted skeleton.
/// `required` refuses a missing or incompatible scene instead of leaving a blank inspect view.
pub(super) fn apply(
    (directory, manager): (&Path, &PackageManager),
    emission: &mut PackageEmission,
    (rigs, supplied): (&[Rig], Option<&rig::Renumbering>),
    table: Option<&[u8]>,
    item: u32,
    companions: &mut linking::Companions,
    required: bool,
) -> AuthoringResult<Option<ReplacementSpec>> {
    let Some(skeleton) = rigs.iter().find(|rig| rig.kind == Kind::Skeleton) else {
        return unavailable(
            required,
            "Inspect scene has no converted equipment skeleton",
        );
    };
    let grown = checked(
        rig::bone_count(&Payload(skeleton.converted.clone())),
        "converted skeleton",
    )? > checked(rig::bone_count(&skeleton.original), "stock skeleton")?;
    if !grown {
        return Ok(None);
    }
    let ordinal = strings_ordinal(emission, item)?;
    let (bucket, first, second) = item_string_type_keys(&emission.host_new_tags[ordinal].payload)?;
    // Stock rows only: a row an import added this build names a pair that is not native yet.
    let stock = Table::parse(&read(manager, TABLE.0)?)?;
    let Some(current) = stock.row(bucket, first).cloned() else {
        return unavailable(
            required,
            "Inspect scene has no stock row for the authored item",
        );
    };
    // Weapon rows carry one style row, whose key names the scene object that shows the weapon.
    let [style] = current.subs.as_slice() else {
        return unavailable(required, "Inspect scene has no single weapon style row");
    };
    let renumbering = mapping(skeleton, supplied)?;
    // The item's scene, then the scenes of its bucket and the rest. A stock scene of another
    // layout, or whose weapon object cannot take the converted rigs, is no candidate.
    let rows = stock.rows();
    let mut seen = BTreeSet::new();
    let mut source = None;
    for row in std::iter::once(&current)
        .chain(rows.iter().filter(|row| row.bucket == bucket))
        .chain(rows.iter().filter(|row| row.bucket != bucket))
    {
        let [sub] = row.subs.as_slice() else {
            continue;
        };
        let shown = read_u32(sub, 0)?;
        if !seen.insert((row.pair, shown)) {
            continue;
        }
        let Ok(candidate) = scene(manager, row.pair) else {
            continue;
        };
        let Some(object) = weapon_object(manager, &candidate, shown, skeleton.native)? else {
            continue;
        };
        match copies(manager, &candidate, object, rigs, &renumbering) {
            Ok(copies) => {
                source = Some((candidate, shown, copies));
                break;
            }
            Err(error) => eprintln!(
                "Inspect scene {:08X} cannot take the converted rigs: {error}",
                row.pair
            ),
        }
    }
    let Some((source, shown, copies)) = source else {
        eprintln!(
            "No crosshair scene shows a piece with skeleton {:08X}. The inspect screen keeps scene {:08X}",
            skeleton.native, current.pair
        );
        return unavailable(
            required,
            "No compatible inspect scene can bind the grown equipment rig",
        );
    };
    let nodes = nodes((directory, manager), &source, copies, companions)?;
    let linked = linking::link(
        directory,
        emission,
        manager,
        nodes,
        PAIR,
        companions,
        0,
        |_, _| Ok(()),
        None,
    )?;
    let pair = linked.symbols[PAIR];
    let mut table = match table {
        Some(data) => Table::parse(data)?,
        None => stock,
    };
    let (hash, key) = (0u32..)
        .map(|salt| {
            (
                fnv1(&format!("inspect_row_{item:08x}_{salt}")),
                fnv1(&format!("inspect_{item:08x}_{salt}")),
            )
        })
        .find(|&(hash, key)| {
            ![0, u32::MAX, 0x811C_9DC5].contains(&key)
                && !table
                    .rows()
                    .iter()
                    .any(|row| row.hash == hash || (row.bucket == bucket && row.key == key))
        })
        .ok_or_else(|| invalid("Inspect scene row has no free key"))?;
    // The rest of the style row follows the bucket, so only its key changes.
    let mut style = *style;
    style[..4].copy_from_slice(&shown.to_le_bytes());
    table.insert(Row {
        hash,
        bucket,
        key,
        pair: pair.0,
        subs: vec![style],
    })?;
    set_item_string_type_keys(&mut emission.host_new_tags[ordinal].payload, key, second)?;
    Ok(Some(ReplacementSpec {
        tag: TABLE,
        payload: table.serialize()?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton(names: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0; 0xF0 + names.len() * 16];
        bytes[24..32].copy_from_slice(&0x28u64.to_le_bytes());
        bytes[0x3C..0x40].copy_from_slice(&0x8080_8546u32.to_le_bytes());
        bytes[0xC0..0xC8].copy_from_slice(&(names.len() as u64).to_le_bytes());
        bytes[0xC8..0xD0].copy_from_slice(&0x18u64.to_le_bytes());
        bytes[0xE0..0xE8].copy_from_slice(&(names.len() as u64).to_le_bytes());
        bytes[0xE8..0xEC].copy_from_slice(&0x8080_8A08u32.to_le_bytes());
        for (index, name) in names.iter().enumerate() {
            bytes[0xF0 + index * 16..0xF4 + index * 16].copy_from_slice(&name.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn inspect_rig_mapping_retains_bones_moved_after_appended_range() {
        let rig = Rig {
            native: 0x80BC_1000,
            kind: Kind::Skeleton,
            original: Payload(skeleton(&[10, 20, 30])),
            converted: skeleton(&[10, 40, 50, 20, 30]),
            relocation: None,
        };
        let expected = rig::Renumbering::inserted(3, 2, 1);
        let inferred = mapping(&rig, None).unwrap();
        let supplied = mapping(&rig, Some(&expected)).unwrap();
        assert_eq!(inferred.map, [0, 3, 4, 1, 2]);
        assert_eq!(supplied.map, inferred.map);
        assert_eq!(supplied.first_appended, 1);
        assert!(mapping(&rig, Some(&rig::Renumbering::appended(3, 2))).is_err());
    }

    #[test]
    fn inspect_scene_relinks_every_copy_without_touching_shared_words() {
        let words = [0x80BC_1000u32, 0x80BC_2000, 0x8FFF_FF00, 0x80BC_1000];
        let bytes = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let names = BTreeMap::from([
            (words[0], "skeleton".to_owned()),
            (words[2], "controls".to_owned()),
        ]);
        let copy = node("piece", 0x80BC_3000, bytes, &names).unwrap();
        assert_eq!(
            copy.patches,
            [
                (0, "skeleton".to_owned()),
                (8, "controls".to_owned()),
                (12, "skeleton".to_owned())
            ]
        );
        assert_eq!(read_u32(&copy.payload, 4).unwrap(), words[1]);
        for offset in [0, 8, 12] {
            assert_eq!(read_u32(&copy.payload, offset).unwrap(), u32::MAX);
        }
    }
}
