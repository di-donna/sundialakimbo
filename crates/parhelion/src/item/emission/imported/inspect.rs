//! Read imported graph metadata for the shared grown-rig inspect scene builder.
use super::*;
use crate::item::emission::inspect_scene::{self, Kind, Rig};
use parhelion_import::tiger::payload::Payload;

/// The equipment rigs the weapon's runtime entity binds in place of its stock ones. They apply
/// only with linked first-person animation, as on the runtime entity.
fn rigs(graph: &Value, folder: &Path) -> AuthoringResult<Vec<Rig>> {
    let animation = &graph["animation"];
    if animation["first_person_status"] != "linked" {
        return Ok(Vec::new());
    }
    let file = |value: &Value| -> AuthoringResult<Vec<u8>> {
        let name = value
            .as_str()
            .ok_or_else(|| invalid("Imported rig file is missing"))?;
        fs::read(folder.join(name)).map_err(|e| invalid(format!("Imported rig {name}: {e}")))
    };
    let mut rigs = Vec::new();
    for value in animation["first_person"]["rigs"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if value["first_person"] != false {
            continue;
        }
        let kind = match value["kind"].as_str() {
            None | Some("skeleton") => Kind::Skeleton,
            Some("controls") => Kind::Controls,
            Some("markers") => Kind::Markers,
            Some(_) => return Err(invalid("Imported rig owner kind is unsupported")),
        };
        let relocation = &value["relocation"];
        let relocation = if relocation.is_null() {
            None
        } else {
            let field = |key: &str| {
                relocation[key]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| invalid("Imported rig relocation is incomplete"))
            };
            Some((field("cut")?, field("delta")?))
        };
        rigs.push(Rig {
            native: value["native"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| invalid("Imported rig owner is missing"))?,
            kind,
            original: Payload(file(&value["template_file"])?),
            converted: file(&value["file"])?,
            relocation,
        });
    }
    Ok(rigs)
}

pub(super) fn apply(
    source: (&Path, &PackageManager),
    emission: &mut PackageEmission,
    (graph, folder): (&Value, &Path),
    table: Option<&[u8]>,
    item: u32,
    companions: &mut linking::Companions,
) -> AuthoringResult<Option<ReplacementSpec>> {
    let rigs = rigs(graph, folder)?;
    inspect_scene::apply(
        source,
        emission,
        (&rigs, None),
        table,
        item,
        companions,
        false,
    )
}
