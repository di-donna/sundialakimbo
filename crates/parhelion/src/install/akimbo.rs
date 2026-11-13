//! The native binding file shares the package journal, backup, publish and rollback transaction.
use super::*;
use crate::weapon::akimbo::deployment::{self, Descriptor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Bindings {
    pub metadata: ArtifactMetadata,
    pub descriptor: Descriptor,
}

impl Bindings {
    pub(super) fn verify(&self, stage: &Path, packages: &Path) -> Result<(), String> {
        let path = stage.join(deployment::FILE_NAME);
        sundial::package_authoring::mod_content::checked_path(stage, &path)?;
        let bytes = deployment::bounded_read(&path, deployment::BYTE_LIMIT)?;
        let digest = crate::artifact::digest_bytes(&bytes);
        if digest.byte_length != self.metadata.byte_length || digest.sha256 != self.metadata.sha256
        {
            return Err("The staged akimbo descriptor changed after review".into());
        }
        self.descriptor.verify_runtime(packages)?;
        self.descriptor.verify_sources(packages)
    }
}

pub(super) fn validate(
    stage: &Path,
    packages: &Path,
    metadata: Option<ArtifactMetadata>,
    sources: &[ArtifactMetadata],
    artifacts: &[ArtifactMetadata],
    paired_recipes: usize,
) -> Result<Option<Bindings>, InstallError> {
    let Some(metadata) = metadata else {
        if paired_recipes != 0 {
            return Err(InstallError::validation(
                "An akimbo recipe has no native binding descriptor",
            ));
        }
        if stage
            .join(deployment::FILE_NAME)
            .try_exists()
            .map_err(|e| InstallError::validation(e.to_string()))?
        {
            return Err(InstallError::validation(
                "Unlisted akimbo descriptor in staging run",
            ));
        }
        return Ok(None);
    };
    if metadata.file_name != deployment::FILE_NAME
        || metadata.byte_length == 0
        || metadata.byte_length > deployment::BYTE_LIMIT
    {
        return Err(InstallError::validation(
            "Invalid staged akimbo descriptor metadata",
        ));
    }
    let path = stage.join(deployment::FILE_NAME);
    sundial::package_authoring::mod_content::checked_path(stage, &path)
        .map_err(InstallError::validation)?;
    let descriptor = Descriptor::read(&path).map_err(InstallError::validation)?;
    if descriptor.profiles.len() != paired_recipes {
        return Err(InstallError::validation(
            "Native akimbo profile count differs from the selected akimbo recipes",
        ));
    }
    descriptor
        .verify_package_metadata(sources, artifacts)
        .map_err(InstallError::validation)?;
    let bindings = Bindings {
        metadata,
        descriptor,
    };
    bindings
        .verify(stage, packages)
        .map_err(InstallError::validation)?;
    Ok(Some(bindings))
}

/// The name is reserved by this protocol. Unknown or malformed bytes are never removed as ours.
pub(super) fn installed(packages: &Path) -> Result<Option<ArtifactMetadata>, InstallError> {
    let path = packages.join(deployment::FILE_NAME);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(InstallError::validation(error.to_string())),
        Ok(_) => {}
    }
    sundial::package_authoring::mod_content::checked_path(packages, &path)
        .map_err(InstallError::validation)?;
    Descriptor::validate_installed_file(&path)
        .map(Some)
        .map_err(InstallError::validation)
}

pub(super) fn verify_runtime(validated: &ValidatedRun) -> Result<(), String> {
    if let Some(bindings) = &validated.akimbo {
        bindings.verify(
            &validated.staged_run_directory,
            &validated.target_packages_directory,
        )?;
    }
    Ok(())
}

pub(super) fn verify_installed(validated: &ValidatedRun) -> Result<(), String> {
    verify_runtime(validated)?;
    if let Some(bindings) = &validated.akimbo {
        for package in bindings.descriptor.packages.iter() {
            let file_name = package
                .path
                .strip_prefix("packages/")
                .ok_or("Invalid package pin")?;
            let path = validated.target_packages_directory.join(file_name);
            sundial::package_authoring::mod_content::checked_path(
                &validated.target_packages_directory,
                &path,
            )?;
            let digest = digest_file(&path).map_err(|error| error.to_string())?;
            if !package.sha256.eq_ignore_ascii_case(&digest.sha256) {
                return Err(format!("Installed akimbo package pin changed: {file_name}"));
            }
        }
    }
    Ok(())
}
