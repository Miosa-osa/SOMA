//! The build pipeline both prepare examples run.
//!
//! Importing the layout, normalizing the rootfs, compiling the Generation, and writing the
//! prepared-store entry are identical whether the Machine shape came from the command line or
//! from a Template document. Only the step that decides the Template revision differs, so that
//! step is the caller's closure and everything around it lives here once.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use soma::OciPlatform;
use soma_generation::{
    BuildHost, CompileGeneration, CompilerProfile, ImportLimits, ImportOciLayout, MachineInputs,
    NormalizeOciRootfs, NormalizedRootfs, OciSelection, RootfsLimits, TemplateRevision, Toolchain,
    compile_generation, generation_manifest::encode_candidate, import_oci_layout,
    normalize_oci_rootfs,
};

use crate::publication::Publication;

/// The build inputs a prepared-store entry is compiled from.
pub(crate) struct BuildInputs {
    pub(crate) layout: PathBuf,
    pub(crate) kernel: PathBuf,
    pub(crate) kernel_config: PathBuf,
    pub(crate) agent: PathBuf,
    pub(crate) erofs_tools: PathBuf,
    pub(crate) e2fsprogs: PathBuf,
    pub(crate) out_entry: PathBuf,
}

/// What one prepared entry ended up holding.
pub(crate) struct Prepared {
    pub(crate) reference: String,
    pub(crate) candidate_id: String,
    pub(crate) entry_count: u32,
}

/// Compiles one OCI layout into a prepared-store entry.
///
/// `decide` receives the normalized rootfs and the path of the store it was written to, which is
/// what a Template needs in order to ask whether the base image carries its command's program,
/// and returns the revision the compiler will bind.
///
/// # Errors
///
/// Returns the first failure of input checking, import, normalization, the caller's decision,
/// compilation, or publication. A failed run leaves no entry behind.
pub(crate) fn prepare<Decide>(
    inputs: &BuildInputs,
    decide: Decide,
) -> Result<Prepared, Box<dyn Error>>
where
    Decide: FnOnce(&NormalizedRootfs, &Path) -> Result<TemplateRevision, Box<dyn Error>>,
{
    require_present(&inputs.layout.join("oci-layout"), "OCI layout", false)?;
    require_present(&inputs.kernel, "kernel", false)?;
    require_present(&inputs.kernel_config, "kernel configuration", false)?;
    require_present(&inputs.agent, "guest agent", false)?;
    require_present(&inputs.erofs_tools, "erofs tools directory", true)?;
    require_present(&inputs.e2fsprogs, "e2fsprogs directory", true)?;

    let publication = Publication::begin(&inputs.out_entry)?;
    let store = publication.path().join("store");
    let staging = publication.path().join("staging");
    fs::create_dir_all(&store)?;
    fs::create_dir_all(&staging)?;

    let platform = OciPlatform::new("linux", "amd64", None)?;
    let imported = import_oci_layout(ImportOciLayout::new(
        &inputs.layout,
        &store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))?;
    let normalized = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &store,
        RootfsLimits::default(),
    ))?;

    let revision = decide(&normalized, &store)?;
    let mut profile = CompilerProfile::v1();
    // A revision with writable storage narrows the profile to the one class it selects, so the
    // compiler builds exactly the template that Generation needs. A revision with none builds
    // no template at all, and the profile's declared classes are left alone because nothing
    // will consult them.
    if revision.writable_storage_bytes() > 0 {
        profile.overlay_capacities = vec![revision.writable_storage_bytes()];
    }

    let compiled = compile_generation(CompileGeneration::new(
        &revision,
        &normalized,
        &store,
        &profile,
        BuildHost::new(
            &staging,
            Toolchain::new(&inputs.erofs_tools, &inputs.e2fsprogs),
            MachineInputs::new(
                &inputs.kernel,
                &inputs.kernel_config,
                &inputs.agent,
                &inputs.agent,
            ),
        ),
    ))?;

    // The entry is what a prepared store holds: the published Candidate bytes, the artifact store
    // those bytes describe, and the reference this entry answers to.
    let reference = revision.image().reference().as_str().to_owned();
    let candidate_bytes = encode_candidate(&compiled.candidate.manifest)?;
    publication.write_private("candidate.somacan", &candidate_bytes)?;
    publication.write_private("reference", reference.as_bytes())?;
    fs::remove_dir_all(&staging)?;
    publication.commit()?;

    Ok(Prepared {
        reference,
        candidate_id: compiled.candidate.id.as_str().to_owned(),
        entry_count: normalized.entry_count(),
    })
}

/// Reports one prepared entry the way the server-setup runbook quotes it.
pub(crate) fn report(prepared: &Prepared, out_entry: &Path) {
    println!(
        "prepared {} at {}\n  candidate id: {}\n  entries: {}",
        prepared.reference,
        out_entry.display(),
        prepared.candidate_id,
        prepared.entry_count,
    );
}

fn require_present(path: &Path, kind: &str, directory: bool) -> Result<(), String> {
    let present = if directory {
        path.is_dir()
    } else {
        path.is_file()
    };
    if present {
        Ok(())
    } else {
        Err(format!("{kind} not found at {}", path.display()))
    }
}
