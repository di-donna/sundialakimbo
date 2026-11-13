//! Versioned native bindings and offline capability verification. DLLs are hashed, never loaded.
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};

use crate::artifact::{ArtifactMetadata, digest_bytes, digest_file};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 5;
pub const GAME_BUILD: u32 = 86657;
pub const FILE_NAME: &str = "parhelion-akimbo-bindings.json";
pub const CAPABILITY_PATH: &str = "Dawn/akimbo/runtime-capability.json";
pub const BYTE_LIMIT: u64 = 256 * 1024;
pub const MAX_PROFILES: usize = 64;
const MAX_PACKAGES: usize = 64;
const MAX_SOURCES: usize = 512;
const CAPABILITY_BYTE_LIMIT: u64 = 16 * 1024;
mod retirement;
mod sources;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_descriptor() -> Descriptor {
    Descriptor {
        schema_version: SCHEMA,
        game_build: GAME_BUILD,
        generation: "1".repeat(32),
        runtime_sha256: "B".repeat(64),
        exe_sha256: "C".repeat(64),
        source_artifacts: Vec::new(),
        packages: vec![Package {
            path: format!(
                "packages/{}",
                crate::package_profile::PARHELION_ASSET_FILE_NAME
            ),
            sha256: "A".repeat(64),
        }],
        profiles: vec![Profile {
            model_tag: tiger_pkg::TagHash::new(
                crate::package_profile::PARHELION_ASSET_PACKAGE_ID,
                1,
            )
            .0,
            adapter: 7,
            pose: None,
            gap_mm: 360,
            reload_policy: 3,
            visual: Some(Visual {
                additional_markers: Vec::new(),
                marker_bones: Vec::new(),
                muzzle_bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
                eject_bits: [0, 0, 0, 1.0f32.to_bits(), 0, 0, 0, 1.0f32.to_bits()],
                tracer_tag: 0x8152c7d0,
                tracer_definition: 8,
                fx: Fx {
                    tag: 0x8152c7d1,
                    bytes: 256,
                    definition: 8,
                    endpoint_binding: 2,
                    selectors: vec![FxSelector {
                        offset: 32,
                        particle: 0x8152c7d2,
                        binding: 1,
                        inputs: 1,
                        markers: 1,
                        groups: 1,
                        program: 0x8152c7d3,
                        program_bytes: 64,
                        role: 1,
                    }],
                },
            }),
            rig: Rig {
                animation_tag: 0x8152c7cd,
                skeleton_tag: 0x80bc1000,
                rig_tag: 0x80bc1001,
            },
            barrel: Barrel {
                core_tag: 0x8152c7c3,
                content_tag: 0x8152c7c5,
                content_definition: 1032,
                root_definition: 13408,
                barrel_definition: 17328,
                core_size: 34944,
                variant: 13,
                projectile: u32::MAX,
                content_size: 13956,
                property_count: 22,
                flags: 0x10,
            },
        }],
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    pub animation_tag: u32,
    pub skeleton_tag: u32,
    pub rig_tag: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Barrel {
    pub core_tag: u32,
    pub content_tag: u32,
    pub content_definition: u32,
    pub root_definition: u32,
    pub barrel_definition: u32,
    pub core_size: u32,
    /// u32::MAX selects the native embedded default; other values pin an indexed row.
    pub variant: u32,
    pub projectile: u32,
    pub content_size: u32,
    pub property_count: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub model_tag: u32,
    pub adapter: u32,
    /// Adapters 10/11's source-derived pose contract. Older adapters keep their fixed layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<GunPose>,
    /// Horizontal distance between hand roots, shared by the geometry and native pose.
    pub gap_mm: u16,
    /// 1 selects empty magazines; 2 selects all non-full magazines; 3 selects the
    /// lower magazine, or both when both are empty.
    pub reload_policy: u32,
    pub rig: Rig,
    pub barrel: Barrel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<Visual>,
}

/// The shared native arm prefix has 72 bones. All appended weapon/control data is bounded
/// independently and proved by the asset compiler; a matching count alone is insufficient.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GunPose {
    pub weapon_bone_count: u32,
    pub marker_bone: u32,
    pub marker_bind_bits: [u32; 8],
    pub control_count: u32,
    /// Serialized frozen-control header size, not the animation asset's file length.
    pub animation_bytes: u32,
}

impl GunPose {
    pub fn validate(&self) -> Result<(), String> {
        if !(3..=56).contains(&self.weapon_bone_count)
            // The marker follows the source's FK bones, before its preserved nonempty
            // procedural range. Native guns have different FK counts.
            || self.marker_bone < 1
            || self.marker_bone >= self.weapon_bone_count - 1
            || self.control_count < 70
            || self.control_count > 72 + self.weapon_bone_count
            || self.animation_bytes < 0x100
            || self.animation_bytes > 0x1000000
            || self.animation_bytes & 7 != 0
        {
            return Err("Akimbo gun pose exceeds its bounded native layout".into());
        }
        let bind = self.marker_bind_bits.map(f32::from_bits);
        let norm = bind[..4].iter().map(|value| value * value).sum::<f32>();
        if !bind.iter().all(|value| value.is_finite())
            || (norm - 1.).abs() > 0.00002
            || (bind[7] - 1.).abs() > 0.000002
            || bind[4..7].iter().any(|value| value.abs() >= 3.)
        {
            return Err("Akimbo gun marker has an invalid bind transform".into());
        }
        Ok(())
    }
}

/// Exact IEEE754 frame bits preserve the package's calibrated visual attachment transform.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visual {
    pub muzzle_bits: [u32; 8],
    pub eject_bits: [u32; 8],
    /// Local attachment groups for selector roles 3 and above, in native marker order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_markers: Vec<Vec<[u32; 8]>>,
    /// Sparse non-root attachment bindings; omitted markers use the weapon root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_bones: Vec<MarkerBone>,
    pub tracer_tag: u32,
    pub tracer_definition: u32,
    pub fx: Fx,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerBone {
    pub role: u32,
    pub index: u32,
    pub bone: u32,
    /// Exact object bind in the source weapon's identity-root space.
    pub bind_bits: [u32; 8],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fx {
    pub tag: u32,
    pub bytes: u32,
    pub definition: u32,
    pub endpoint_binding: u32,
    pub selectors: Vec<FxSelector>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxSelector {
    pub offset: u32,
    pub particle: u32,
    pub binding: u32,
    pub inputs: u32,
    pub markers: u32,
    pub groups: u32,
    pub program: u32,
    pub program_bytes: u32,
    pub role: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema_version: u32,
    pub game_build: u32,
    pub generation: String,
    pub runtime_sha256: String,
    pub exe_sha256: String,
    pub packages: Vec<Package>,
    pub source_artifacts: Vec<Package>,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub schema_version: u32,
    pub game_build: u32,
    pub runtime_sha256: String,
    pub adapter_ids: Vec<u32>,
    pub max_profiles: u32,
    pub descriptor_schema: u32,
}

/// A hash-bound declaration from the native host, not a claim made by the weapon recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledCapability {
    pub capability: Capability,
    pub runtime_sha256: String,
    pub exe_sha256: String,
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_tag(tag: u32) -> bool {
    tag >= 0x8000_0000 && tag != u32::MAX
}

fn private_core_tag(tag: u32) -> bool {
    use crate::package_profile::{
        MAX_AUTHORED_STANDALONE_PACKAGE_ID, MIN_AUTHORED_STANDALONE_PACKAGE_ID,
        PRIVATE_PERK_RUNTIME_EXPECTED_ENTRY_COUNT, PRIVATE_PERK_RUNTIME_PACKAGE_ID,
    };
    let Some(offset) = tag.checked_sub(0x8080_0000) else {
        return false;
    };
    let package = offset >> 13;
    (package == u32::from(PRIVATE_PERK_RUNTIME_PACKAGE_ID)
        && (tag & 0x1fff) as usize >= PRIVATE_PERK_RUNTIME_EXPECTED_ENTRY_COUNT)
        || (u32::from(MIN_AUTHORED_STANDALONE_PACKAGE_ID)
            ..=u32::from(MAX_AUTHORED_STANDALONE_PACKAGE_ID))
            .contains(&package)
}

impl Visual {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_reserved_fx()?;
        let valid_frame = |bits: &[u32; 8]| {
            let values = bits.map(f32::from_bits);
            let norm = values[..4].iter().map(|value| value * value).sum::<f32>();
            values.iter().all(|value| value.is_finite())
                && norm > 0.96
                && norm < 1.04
                && values[4..7].iter().all(|value| value.abs() < 3.0)
                && values[7] > 0.5
                && values[7] < 2.0
        };
        let fx = &self.fx;
        if !valid_frame(&self.muzzle_bits)
            || !valid_frame(&self.eject_bits)
            || self.additional_markers.len() > 14
            || self.marker_bones.len() > 86
            || self.additional_markers.iter().any(|group| {
                group.is_empty() || group.len() > 6 || group.iter().any(|bits| !valid_frame(bits))
            })
            || !valid_tag(self.tracer_tag)
            || self.tracer_definition == 0
            || self.tracer_definition & 7 != 0
            || self.tracer_definition > 0x1000000
            || !valid_tag(fx.tag)
            || fx.bytes == 0
            || fx.bytes > 0x1000000
            || fx.definition & 7 != 0
            || fx
                .bytes
                .checked_sub(fx.definition)
                .is_none_or(|bytes| bytes < 16)
            || fx.endpoint_binding > 255
            || fx.selectors.is_empty()
            || fx.selectors.len() > 32
        {
            return Err("Akimbo visual data is outside the native adapter's checked bounds".into());
        }
        let mut bound_markers = BTreeSet::new();
        for binding in &self.marker_bones {
            let count = match binding.role {
                1 | 2 => Some(1),
                role if role >= 3 => self.additional_markers.get(role as usize - 3).map(Vec::len),
                _ => None,
            };
            if count.is_none_or(|count| binding.index as usize >= count)
                || binding.bone == 0
                || binding.bone >= 56
                || !valid_frame(&binding.bind_bits)
                || f32::from_bits(binding.bind_bits[7]) <= 0.95
                || f32::from_bits(binding.bind_bits[7]) >= 1.05
                || !bound_markers.insert((binding.role, binding.index))
            {
                return Err("Akimbo attachment bone is outside its checked marker group".into());
            }
        }
        let mut offsets = BTreeSet::new();
        let mut has_muzzle = false;
        for selector in &fx.selectors {
            if selector.offset & 7 != 0
                || fx
                    .bytes
                    .checked_sub(selector.offset)
                    .is_none_or(|bytes| bytes < 24)
                || !offsets.insert(selector.offset)
                || !valid_tag(selector.particle)
                || !valid_tag(selector.program)
                || selector.program_bytes == 0
                || selector.program_bytes > 0x1000000
                || selector.binding > 255
                || selector.binding == fx.endpoint_binding
                || !matches!(selector.inputs, 1 | 2)
                || !(1..=6).contains(&selector.markers)
                || selector.groups < selector.inputs
                || selector.groups > 3
                || selector.role == 0
                || selector.role as usize > 2 + self.additional_markers.len()
                || (selector.role <= 2 && selector.markers != 1)
                || (selector.role >= 3
                    && self.additional_markers[selector.role as usize - 3].len()
                        > selector.markers as usize)
            {
                return Err(
                    "Akimbo FX selector is outside the native adapter's checked bounds".into(),
                );
            }
            has_muzzle |= selector.role == 1;
        }
        if !has_muzzle {
            return Err("Akimbo visual data requires a muzzle selector".into());
        }
        Ok(())
    }

    fn validate_reserved_fx(&self) -> Result<(), String> {
        if matches!(
            self.fx.tag,
            0x80ef262f | 0x80ff0531 | 0x80ef2608 | 0x80ef42ba | 0x80ef42e3
        ) {
            return Err(
                "A generated SMG cannot take ownership of another native adapter's FX sequence"
                    .into(),
            );
        }
        if self.fx.tag != 0x80ef5685 {
            return Ok(());
        }
        // This shared native sequence already has a legacy owner. Its checked mapping must
        // agree exactly; newly encountered sequence identities remain data driven.
        const SELECTORS: [[u32; 9]; 12] = [
            [0x4140, 0x80f2f8e3, 0, 2, 1, 3, 0x80bfa72f, 592, 2],
            [0x43b0, 0x80ef5723, 2, 1, 1, 1, 0x80bc111a, 1576, 1],
            [0x4620, 0x80ef5724, 2, 1, 1, 1, 0x80bc111c, 1544, 1],
            [0x4890, 0x80ef5725, 2, 2, 1, 2, 0x80bc111e, 2296, 1],
            [0x4b00, 0x80ef5726, 2, 1, 1, 1, 0x80bc1120, 2392, 1],
            [0x6300, 0x80ef5705, 2, 1, 1, 1, 0x80bc1139, 1800, 1],
            [0x6570, 0x80ef5706, 2, 2, 1, 2, 0x80bc113b, 2588, 1],
            [0x6cc0, 0x80ef5707, 2, 1, 1, 1, 0x80bc113d, 1784, 1],
            [0x6f30, 0x80ef5708, 2, 1, 1, 1, 0x80bc113f, 1804, 1],
            [0x78f0, 0x80ef5709, 2, 1, 1, 1, 0x80bc1141, 2072, 1],
            [0x7b60, 0x80ef570a, 2, 2, 1, 2, 0x80bc1143, 664, 1],
            [0x7dd0, 0x80ef570b, 2, 1, 1, 1, 0x80bc1145, 1736, 1],
        ];
        if self.fx.bytes != 34992
            || self.fx.definition != 0x26f8
            || self.fx.endpoint_binding != 1
            || self.fx.selectors.len() != SELECTORS.len()
            || self
                .fx
                .selectors
                .iter()
                .zip(SELECTORS)
                .any(|(actual, expected)| {
                    [
                        actual.offset,
                        actual.particle,
                        actual.binding,
                        actual.inputs,
                        actual.markers,
                        actual.groups,
                        actual.program,
                        actual.program_bytes,
                        actual.role,
                    ] != expected
                })
        {
            return Err("Akimbo FX binding contradicts the existing native sequence owner".into());
        }
        Ok(())
    }
}

impl Profile {
    fn referenced_tags(&self) -> Vec<u32> {
        let mut tags = vec![
            self.model_tag,
            self.rig.animation_tag,
            self.rig.skeleton_tag,
            self.rig.rig_tag,
            self.barrel.core_tag,
            self.barrel.content_tag,
        ];
        if self.barrel.projectile != u32::MAX {
            tags.push(self.barrel.projectile);
        }
        if let Some(visual) = &self.visual {
            tags.extend([visual.tracer_tag, visual.fx.tag]);
            for selector in &visual.fx.selectors {
                tags.extend([selector.particle, selector.program]);
            }
        }
        tags
    }

    pub fn validate(&self) -> Result<(), String> {
        match (self.adapter, &self.pose) {
            (10 | 11, Some(pose)) => pose.validate()?,
            (10 | 11, None) => {
                return Err("Akimbo generalized gun requires a source-derived pose".into());
            }
            (_, Some(_)) => {
                return Err("This akimbo adapter cannot use a generalized gun pose".into());
            }
            (_, None) => {}
        }
        if !matches!(self.reload_policy, 1..=3) {
            return Err("Unsupported akimbo reload policy".into());
        }
        if !(50..=800).contains(&self.gap_mm) {
            return Err("Akimbo gap must be between 50 and 800 millimetres".into());
        }
        match (self.adapter, &self.visual) {
            (7, Some(visual))
                if !visual.additional_markers.is_empty() || !visual.marker_bones.is_empty() =>
            {
                return Err(
                    "Additional local effect markers require the generalized gun adapter".into(),
                );
            }
            (7, Some(visual))
                if [visual.muzzle_bits[7], visual.eject_bits[7]]
                    .into_iter()
                    .map(f32::from_bits)
                    .any(|scale| !(scale > 0.95 && scale < 1.05)) =>
            {
                return Err(
                    "Scaled local effect attachments require the generalized gun adapter".into(),
                );
            }
            (7 | 10 | 11, Some(visual)) => visual.validate()?,
            (7, None) => return Err("Akimbo SMG requires generated visual bindings".into()),
            (10 | 11, None) => {
                return Err("Akimbo generalized gun requires generated visual bindings".into());
            }
            (_, Some(_)) => {
                return Err("This akimbo adapter does not accept generated visual bindings".into());
            }
            (_, None) => {}
        }
        if let (Some(pose), Some(visual)) = (&self.pose, &self.visual)
            && visual.marker_bones.iter().any(|binding| {
                binding.bone >= pose.weapon_bone_count || binding.bone == pose.marker_bone
            })
        {
            return Err(
                "Akimbo attachment cannot use an absent bone or the private pose marker".into(),
            );
        }
        const LEGACY_MODELS: [u32; 12] = [
            0x81a2958a, 0x81a295e0, 0x81a295fd, 0x81a2961a, 0x81a29815, 0x81a29897, 0x81a2990a,
            0x81a29990, 0x81a29a19, 0x81a29991, 0x81a29a1b, 0x81a29a1d,
        ];
        const RIGS: [(u32, u32, u32, u32); 3] = [
            (0x81579415, 0x80bc8623, 0x80bc8624, 0),
            (0x8152c7cd, 0x80bc1000, 0x80bc1001, 7),
            (0x8161f90f, 0x80bbdb9c, 0x80bbdb9d, 8),
        ];
        let r = &self.rig;
        let b = &self.barrel;
        let fields = [self.model_tag, r.animation_tag, r.skeleton_tag, r.rig_tag];
        if !fields.into_iter().all(valid_tag)
            || fields.into_iter().collect::<BTreeSet<_>>().len() != fields.len()
            || LEGACY_MODELS.contains(&self.model_tag)
            || !matches!(self.adapter, 2 | 5 | 6 | 7 | 8 | 10 | 11)
        {
            return Err("Invalid akimbo model, adapter or rig identity".into());
        }
        for (animation, skeleton, rig, class) in RIGS {
            if [animation, skeleton, rig].contains(&self.model_tag)
                || (r.animation_tag == animation
                    && (r.skeleton_tag != skeleton
                        || r.rig_tag != rig
                        || self.rig_class() != class))
            {
                return Err("Akimbo rig binding conflicts with the native rig".into());
            }
        }
        if !valid_tag(b.core_tag)
            || !valid_tag(b.content_tag)
            || b.core_tag == b.content_tag
            || [b.core_tag, b.content_tag].contains(&self.model_tag)
            || b.core_size > 0x1000000
            || b.content_size > 0x1000000
            || b.root_definition == 0
            || [b.root_definition, b.barrel_definition, b.content_definition]
                .into_iter()
                .any(|offset| offset & 7 != 0)
            || b.core_size
                .checked_sub(b.root_definition)
                .is_none_or(|remaining| remaining < 0x8d0)
            || b.core_size
                .checked_sub(b.barrel_definition)
                .is_none_or(|remaining| remaining < 0xecc)
            || b.content_size
                .checked_sub(b.content_definition)
                .is_none_or(|remaining| remaining < 0x270)
            || b.property_count > 4096
            || (!matches!(self.adapter, 10 | 11)
                && (b.property_count == 0 || b.variant == u32::MAX))
            || (b.variant != u32::MAX && b.variant >= b.property_count)
            || u64::from(b.property_count) * 448
                > u64::from(
                    b.content_size
                        .saturating_sub(b.content_definition)
                        .saturating_sub(0x258),
                )
            || (!valid_tag(b.projectile) && b.projectile != u32::MAX)
            || match self.adapter {
                // The generic wire pins the native value exactly. Firing semantics are
                // assessed separately; inventory family or these raw bits cannot prove them.
                10 | 11 => false,
                7 | 8 => b.flags != 0x10,
                _ => b.flags != 0x14,
            }
        {
            return Err(
                "Akimbo barrel identity is outside the native adapter's checked bounds".into(),
            );
        }
        Ok(())
    }

    fn rig_class(&self) -> u32 {
        match self.adapter {
            7 | 8 => self.adapter,
            10 | 11 => 10,
            _ => 0,
        }
    }
}

pub(crate) fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "{} must be a regular file of at most {limit} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} grew beyond its byte limit", path.display()));
    }
    Ok(bytes)
}

impl Capability {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA
            || self.descriptor_schema != SCHEMA
            || self.game_build != GAME_BUILD
        {
            return Err(
                "The installed akimbo host advertises an unsupported schema or game build".into(),
            );
        }
        if !valid_hex(&self.runtime_sha256, 64)
            || self.max_profiles == 0
            || self.max_profiles as usize > MAX_PROFILES
        {
            return Err(
                "The installed akimbo capability has invalid runtime identity or capacity".into(),
            );
        }
        let adapters = self.adapter_ids.iter().copied().collect::<BTreeSet<_>>();
        if adapters.len() != self.adapter_ids.len()
            || adapters.is_empty()
            || adapters
                .iter()
                .any(|id| !matches!(id, 2 | 5 | 6 | 7 | 8 | 10 | 11))
        {
            return Err(
                "The installed akimbo capability has unsupported or repeated adapters".into(),
            );
        }
        Ok(())
    }
}

impl InstalledCapability {
    pub fn read(packages: &Path) -> Result<Self, String> {
        let root = packages
            .parent()
            .ok_or("Akimbo packages have no game root")?;
        let runtime = sundial::package_authoring::installed_runtime(root)?;
        Self::read_for_runtime(packages, &runtime)
    }

    fn read_for_runtime(
        packages: &Path,
        runtime: &sundial::package_authoring::RuntimeSnapshot,
    ) -> Result<Self, String> {
        let root = packages
            .parent()
            .ok_or("Akimbo packages have no game root")?;
        if runtime.brand() != sundial::package_authoring::RuntimeBrand::Dawn
            || runtime.uses_mod_loader()?
        {
            return Err("This akimbo host protocol requires a compatible Dawn runtime and direct package layout".into());
        }
        // Follow the game's local DLL precedence, even when the DLL itself is linked to a build.
        let runtime_directory = if root.join("steam_api64.dll").is_file() {
            root.to_path_buf()
        } else {
            root.join("bin/x64")
        };
        let capability_path = runtime_directory.join(CAPABILITY_PATH);
        sundial::package_authoring::mod_content::checked_path(root, &capability_path)?;
        let capability: Capability =
            serde_json::from_slice(&bounded_read(&capability_path, CAPABILITY_BYTE_LIMIT)?)
                .map_err(|error| format!("Invalid native akimbo capability: {error}"))?;
        capability.validate()?;
        if !capability
            .runtime_sha256
            .eq_ignore_ascii_case(runtime.dll_sha256())
        {
            return Err("The akimbo capability belongs to a different runtime DLL; rebuild or install the matching host capability".into());
        }
        let exe = root.join("destiny2.exe");
        sundial::package_authoring::mod_content::checked_path(root, &exe)?;
        let exe_sha256 = digest_file(&exe)
            .map_err(|error| format!("Could not hash Destiny executable: {error}"))?
            .sha256;
        Ok(Self {
            capability,
            runtime_sha256: runtime.dll_sha256().to_ascii_uppercase(),
            exe_sha256,
        })
    }

    pub fn require_profiles(&self, profiles: &[Profile]) -> Result<(), String> {
        self.capability.validate()?;
        if profiles.is_empty() || profiles.len() > self.capability.max_profiles as usize {
            return Err("The akimbo profile count is outside the installed host's capacity".into());
        }
        for profile in profiles {
            profile.validate()?;
            if !self.capability.adapter_ids.contains(&profile.adapter) {
                return Err(format!(
                    "The installed akimbo host does not support adapter {}",
                    profile.adapter
                ));
            }
        }
        Ok(())
    }
}

impl Descriptor {
    /// Recognize an existing descriptor only so its exact bytes can be backed up and retired.
    /// This never returns an active profile or upgrades a descriptor in place.
    pub(crate) fn validate_installed_file(path: &Path) -> Result<ArtifactMetadata, String> {
        let bytes = bounded_read(path, BYTE_LIMIT)?;
        retirement::validate(&bytes)?;
        let digest = digest_bytes(&bytes);
        Ok(ArtifactMetadata {
            file_name: FILE_NAME.into(),
            byte_length: digest.byte_length,
            sha256: digest.sha256,
        })
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let descriptor: Self = serde_json::from_slice(&bounded_read(path, BYTE_LIMIT)?)
            .map_err(|error| format!("Invalid akimbo descriptor: {error}"))?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA || self.game_build != GAME_BUILD {
            return Err("Unsupported akimbo descriptor schema or game build".into());
        }
        if !valid_hex(&self.generation, 32)
            || !valid_hex(&self.runtime_sha256, 64)
            || !valid_hex(&self.exe_sha256, 64)
        {
            return Err("Invalid akimbo generation or executable/runtime digest".into());
        }
        if self.profiles.is_empty()
            || self.profiles.len() > MAX_PROFILES
            || self.packages.is_empty()
            || self.packages.len() > MAX_PACKAGES
            || self.source_artifacts.len() > MAX_SOURCES
        {
            return Err("Akimbo descriptor exceeds profile/package bounds or is empty".into());
        }
        let mut names = BTreeSet::new();
        for package in self.packages.iter().chain(&self.source_artifacts) {
            let name = package
                .path
                .strip_prefix("packages/")
                .ok_or("Akimbo package path is outside packages")?;
            if name.is_empty()
                || name.contains(['/', '\\', ':'])
                || !name.ends_with(".pkg")
                || name.contains("..")
                || !names.insert(name.to_ascii_lowercase())
                || !valid_hex(&package.sha256, 64)
            {
                return Err("Invalid or repeated akimbo package pin".into());
            }
        }
        let mut models = BTreeSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !models.insert(profile.model_tag) {
                return Err("Repeated akimbo model identity".into());
            }
            if self.profiles.iter().any(|other| {
                other.rig.animation_tag == profile.rig.animation_tag
                    && (other.rig != profile.rig || other.rig_class() != profile.rig_class())
            }) {
                return Err("Conflicting generated akimbo rig bindings".into());
            }
            if let Some(visual) = &profile.visual
                && self
                    .profiles
                    .iter()
                    .filter_map(|other| other.visual.as_ref())
                    .any(|other| other.fx.tag == visual.fx.tag && other.fx != visual.fx)
            {
                return Err("Conflicting generated akimbo FX bindings".into());
            }
        }
        Ok(())
    }

    pub fn verify_runtime(&self, packages: &Path) -> Result<(), String> {
        self.validate()?;
        let installed = InstalledCapability::read(packages)?;
        self.verify_identity(&installed)
    }

    pub(crate) fn verify_sources(&self, packages: &Path) -> Result<(), String> {
        sources::verify(self, packages)
    }

    fn verify_identity(&self, installed: &InstalledCapability) -> Result<(), String> {
        installed.require_profiles(&self.profiles)?;
        if !self
            .runtime_sha256
            .eq_ignore_ascii_case(&installed.runtime_sha256)
            || !self.exe_sha256.eq_ignore_ascii_case(&installed.exe_sha256)
        {
            return Err("The akimbo generation was built for different runtime or executable bytes; rebuild it".into());
        }
        Ok(())
    }

    pub(crate) fn verify_package_metadata(
        &self,
        sources: &[ArtifactMetadata],
        artifacts: &[ArtifactMetadata],
    ) -> Result<(), String> {
        let expected = package_pins(artifacts)?;
        let expected_sources = package_pins(sources)?;
        if self.packages.len() != expected.len()
            || self
                .packages
                .iter()
                .zip(&expected)
                .any(|(actual, expected)| {
                    actual.path != expected.path
                        || !actual.sha256.eq_ignore_ascii_case(&expected.sha256)
                })
        {
            return Err("Akimbo package pins do not match the complete staged generation and stock source set".into());
        }
        if expected_sources.iter().any(|expected| {
            !self.source_artifacts.iter().any(|actual| {
                actual.path == expected.path && actual.sha256.eq_ignore_ascii_case(&expected.sha256)
            })
        }) {
            return Err(
                "Akimbo dependency pins omit or differ from a canonical stock source".into(),
            );
        }
        let generated = artifacts
            .iter()
            .filter_map(|a| crate::package_profile::authored_package_for_file_name(&a.file_name))
            .map(|p| p.package_id)
            .collect::<BTreeSet<_>>();
        if self
            .profiles
            .iter()
            .any(|p| !generated.contains(&tiger_pkg::TagHash(p.model_tag).pkg_id()))
        {
            return Err("An akimbo model is outside the generated package set".into());
        }
        if self.profiles.iter().any(|profile| {
            profile.adapter == 11
                && (!private_core_tag(profile.barrel.core_tag)
                    || !generated.contains(&tiger_pkg::TagHash(profile.barrel.core_tag).pkg_id()))
        }) {
            return Err(
                "A finite-burst akimbo core is outside the private generated entry set".into(),
            );
        }
        Ok(())
    }
}

fn package_pins(artifacts: &[ArtifactMetadata]) -> Result<Vec<Package>, String> {
    let mut pins = std::collections::BTreeMap::new();
    for artifact in artifacts {
        let name = &artifact.file_name;
        if name.contains(['/', '\\', ':'])
            || !name.ends_with(".pkg")
            || !valid_hex(&artifact.sha256, 64)
        {
            return Err("Invalid package metadata for akimbo generation".into());
        }
        if pins
            .insert(name.clone(), artifact.sha256.to_ascii_uppercase())
            .is_some()
        {
            return Err("Repeated package metadata for akimbo generation".into());
        }
    }
    Ok(pins
        .into_iter()
        .map(|(name, sha256)| Package {
            path: format!("packages/{name}"),
            sha256,
        })
        .collect())
}

/// Called only after the compiler supplied actual emitted bindings, never from editable intent.
pub(crate) fn stage(
    packages: &Path,
    directory: &Path,
    sources: &[ArtifactMetadata],
    artifacts: &[ArtifactMetadata],
    profiles: &[Profile],
    source_tags: &[u32],
    expected: Option<&InstalledCapability>,
) -> Result<Option<ArtifactMetadata>, String> {
    if profiles.is_empty() {
        return Ok(None);
    }
    let installed = InstalledCapability::read(packages)?;
    if Some(&installed) != expected {
        return Err("The native akimbo host changed after the build snapshot".into());
    }
    installed.require_profiles(profiles)?;
    let mut descriptor = Descriptor {
        schema_version: SCHEMA,
        game_build: GAME_BUILD,
        generation: "0".repeat(32),
        runtime_sha256: installed.runtime_sha256,
        exe_sha256: installed.exe_sha256,
        packages: package_pins(artifacts)?,
        source_artifacts: sources::collect(packages, sources, artifacts, profiles, source_tags)?,
        profiles: profiles.to_vec(),
    };
    descriptor.validate()?;
    descriptor.verify_package_metadata(sources, artifacts)?;
    let seed = serde_json::to_vec(&descriptor).map_err(|error| error.to_string())?;
    descriptor.generation = digest_bytes(&seed).sha256[..32].to_owned();
    let bytes = serde_json::to_vec_pretty(&descriptor).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > BYTE_LIMIT {
        return Err("Akimbo descriptor exceeds its byte limit".into());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(FILE_NAME))
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    let digest = digest_bytes(&bytes);
    Ok(Some(ArtifactMetadata {
        file_name: FILE_NAME.into(),
        byte_length: digest.byte_length,
        sha256: digest.sha256,
    }))
}
