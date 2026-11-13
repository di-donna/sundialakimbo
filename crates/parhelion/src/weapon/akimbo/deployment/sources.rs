//! Pin canonical inputs and the stock patch chains actually visited by the akimbo compiler.
//! This is runtime dependency integrity, not a claim that every stock package was compiled.
use super::*;
use crate::package_profile::{
    PACKAGE_HEADER_PREFIX_SIZE, PackageHeaderPrefix, SHADOWKEEP_HEADER_VERSION,
    authored_package_for_file_name,
};
use std::collections::BTreeMap;

struct StockFile {
    path: std::path::PathBuf,
    header: PackageHeaderPrefix,
}

fn inventory(packages: &Path) -> Result<BTreeMap<String, StockFile>, String> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(packages).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !crate::artifact::has_pkg_extension(&path) {
            continue;
        }
        sundial::package_authoring::mod_content::checked_path(packages, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Akimbo source is not a regular package: {}",
                path.display()
            ));
        }
        let mut prefix = [0; PACKAGE_HEADER_PREFIX_SIZE];
        fs::File::open(&path)
            .and_then(|mut file| file.read_exact(&mut prefix))
            .map_err(|e| {
                format!(
                    "Could not read akimbo dependency header {}: {e}",
                    path.display()
                )
            })?;
        let header = PackageHeaderPrefix::parse(&prefix);
        if header.version != SHADOWKEEP_HEADER_VERSION {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Akimbo package filename is not Unicode")?;
        if header.build_signature == crate::SUNDIAL_BUILD_SIGNATURE {
            let known = authored_package_for_file_name(&name).is_some_and(|profile| {
                profile.package_id == header.package_id && profile.patch_id == header.patch_id
            });
            if !known {
                return Err(format!(
                    "Unrecognized authored package in akimbo dependency view: {name}"
                ));
            }
            continue;
        }
        files.insert(name, StockFile { path, header });
    }
    Ok(files)
}

fn emitted_ids(artifacts: &[Package]) -> Result<BTreeSet<u16>, String> {
    artifacts
        .iter()
        .map(|artifact| {
            let name = artifact
                .path
                .strip_prefix("packages/")
                .ok_or("Invalid akimbo package path")?;
            authored_package_for_file_name(name)
                .map(|profile| profile.package_id)
                .ok_or_else(|| format!("Unknown authored akimbo package: {name}"))
        })
        .collect()
}

pub(super) fn collect(
    packages: &Path,
    canonical: &[ArtifactMetadata],
    artifacts: &[ArtifactMetadata],
    profiles: &[Profile],
    tracked_tags: &[u32],
) -> Result<Vec<Package>, String> {
    let files = inventory(packages)?;
    let emitted = emitted_ids(&package_pins(artifacts)?)?;
    let expected = canonical
        .iter()
        .map(|a| (a.file_name.as_str(), a))
        .collect::<BTreeMap<_, _>>();
    let mut required = BTreeSet::new();
    for name in expected.keys() {
        required.insert(
            files
                .get(*name)
                .ok_or_else(|| format!("Missing canonical akimbo source: {name}"))?
                .header
                .package_id,
        );
    }
    for tag in profiles
        .iter()
        .flat_map(Profile::referenced_tags)
        .chain(tracked_tags.iter().copied())
    {
        if !valid_tag(tag) {
            return Err(format!("Invalid tracked akimbo source tag {tag:08X}"));
        }
        let id = tiger_pkg::TagHash(tag).pkg_id();
        let has_stock = files.values().any(|file| file.header.package_id == id);
        if !has_stock && !emitted.contains(&id) {
            return Err(format!(
                "Akimbo dependency {tag:08X} has no stock chain or emitted package"
            ));
        }
        if has_stock {
            required.insert(id);
        }
    }
    let mut pins = Vec::new();
    for (name, file) in files {
        if !required.contains(&file.header.package_id) {
            continue;
        }
        let digest = digest_file(&file.path).map_err(|e| e.to_string())?;
        if expected.get(name.as_str()).is_some_and(|expected| {
            expected.byte_length != digest.byte_length
                || !expected.sha256.eq_ignore_ascii_case(&digest.sha256)
        }) {
            return Err(format!(
                "Canonical akimbo input changed before staging: {name}"
            ));
        }
        pins.push(Package {
            path: format!("packages/{name}"),
            sha256: digest.sha256,
        });
        if pins.len() > MAX_SOURCES {
            return Err("Akimbo runtime dependencies exceed the source pin capacity".into());
        }
    }
    Ok(pins)
}

pub(super) fn verify(descriptor: &Descriptor, packages: &Path) -> Result<(), String> {
    let files = inventory(packages)?;
    let emitted = emitted_ids(&descriptor.packages)?;
    let mut pinned_ids = BTreeSet::new();
    let mut pinned_names = BTreeSet::new();
    for pin in &descriptor.source_artifacts {
        let name = pin
            .path
            .strip_prefix("packages/")
            .ok_or("Invalid akimbo source path")?;
        let file = files
            .get(name)
            .ok_or_else(|| format!("Pinned akimbo source is absent or no longer stock: {name}"))?;
        let digest = digest_file(&file.path).map_err(|e| e.to_string())?;
        if !pin.sha256.eq_ignore_ascii_case(&digest.sha256) {
            return Err(format!("Akimbo runtime dependency changed: {name}"));
        }
        pinned_ids.insert(file.header.package_id);
        pinned_names.insert(name);
    }
    for (name, file) in &files {
        if (pinned_ids.contains(&file.header.package_id)
            || emitted.contains(&file.header.package_id))
            && !pinned_names.contains(name.as_str())
        {
            return Err(format!(
                "Unpinned package appeared in an akimbo source patch chain: {name}"
            ));
        }
    }
    for tag in descriptor
        .profiles
        .iter()
        .flat_map(Profile::referenced_tags)
    {
        let id = tiger_pkg::TagHash(tag).pkg_id();
        if !pinned_ids.contains(&id) && !emitted.contains(&id) {
            return Err(format!(
                "Akimbo native reference {tag:08X} has no pinned package"
            ));
        }
    }
    Ok(())
}
