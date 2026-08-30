//! Compiles a real Generation for the live sandbox proof: exports an image from Docker into
//! an OCI layout, imports and normalizes it, and runs the production compiler with the pinned
//! kernel and the built static guest agent.
//! No secret is a compiler input; the guest responder authority is fresh per Instance.

use std::{
    env,
    fs::{self, File},
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sha2::{Digest as _, Sha256};
use soma::{MachineShape, OciImage, OciPlatform};
use soma_generation::{
    BuildHost, CandidateId, CompileGeneration, CompilerProfile, GenerationManifest, ImportLimits,
    ImportOciLayout, LifetimeLimits, MachineInputs, NormalizeOciRootfs, OciSelection, RootfsLimits,
    StartupBehavior, TemplateImage, TemplateRevision, Toolchain, compile_generation,
    import_oci_layout, normalize_oci_rootfs,
};

const MIB: u64 = 1024 * 1024;

/// Every host input the compiler needs, resolved from the environment with explicit failures.
pub struct Inputs {
    pub kernel: PathBuf,
    pub kernel_config: PathBuf,
    pub agent: PathBuf,
    pub erofs_tools: PathBuf,
    pub e2fsprogs: PathBuf,
}

fn default_kernel_config(kernel: &Path) -> PathBuf {
    let sibling = kernel.with_file_name("final.config");
    if sibling.is_file() {
        return sibling;
    }
    kernel
        .parent()
        .and_then(Path::parent)
        .map_or(sibling, |dir| dir.join("config-x86_64-soma-v1"))
}

pub fn inputs(kernel: PathBuf) -> Inputs {
    let kernel_config = env::var_os("SOMA_X86_64_KERNEL_CONFIG")
        .map_or_else(|| default_kernel_config(&kernel), PathBuf::from);
    assert!(
        kernel_config.is_file(),
        "prerequisite failed: kernel configuration text not found at {}; set SOMA_X86_64_KERNEL_CONFIG",
        kernel_config.display()
    );
    let agent = env::var_os("SOMA_GUEST_AGENT").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/x86_64-unknown-linux-musl/release/soma-guest-agent")
        },
        PathBuf::from,
    );
    assert!(
        agent.is_file(),
        "prerequisite failed: static guest agent not found at {}; run scripts/build-guest-agent.sh or set SOMA_GUEST_AGENT",
        agent.display()
    );
    let erofs_tools = env::var_os("SOMA_EROFS_TOOLS").map(PathBuf::from).expect(
        "prerequisite failed: SOMA_EROFS_TOOLS must name the pinned erofs-utils 1.9.4 directory",
    );
    assert!(
        erofs_tools.join("mkfs.erofs").is_file(),
        "prerequisite failed: {} has no mkfs.erofs",
        erofs_tools.display()
    );
    let e2fsprogs = env::var_os("SOMA_E2FSPROGS").map_or_else(
        || {
            ["/usr/sbin", "/sbin"]
                .into_iter()
                .map(PathBuf::from)
                .find(|dir| dir.join("mke2fs").is_file())
                .expect("prerequisite failed: mke2fs not found; set SOMA_E2FSPROGS")
        },
        PathBuf::from,
    );
    Inputs {
        kernel,
        kernel_config,
        agent,
        erofs_tools,
        e2fsprogs,
    }
}

/// Exports `image` from the local Docker engine into an OCI layout under `dir`, or returns the
/// layout named by `override_var`; `None` when Docker cannot export it.
pub fn oci_layout(image: &str, override_var: &str, dir: &Path) -> Option<PathBuf> {
    if let Some(layout) = env::var_os(override_var) {
        let layout = PathBuf::from(layout);
        assert!(
            layout.join("oci-layout").is_file() && layout.join("index.json").is_file(),
            "{override_var} must name an OCI image layout"
        );
        return Some(layout);
    }
    let layout = dir.join(format!("oci-{}", image.replace(['/', ':'], "-")));
    if layout.join("index.json").is_file() {
        return Some(layout);
    }
    let _ = fs::remove_dir_all(&layout);
    fs::create_dir_all(&layout).ok()?;
    let mut save = Command::new("docker")
        .args(["save", image])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    let extract = Command::new("tar")
        .args(["-x", "-C"])
        .arg(&layout)
        .stdin(save.stdout.take()?)
        .status()
        .ok()?;
    let saved = save.wait().ok()?;
    if !saved.success() || !extract.success() || !layout.join("oci-layout").is_file() {
        eprintln!("docker save {image} did not produce an OCI layout");
        let _ = fs::remove_dir_all(&layout);
        return None;
    }
    Some(layout)
}

/// One compiled Generation Candidate and the store that holds its artifacts.
///
/// Everything here is reconstructible from the store, so a compiled Generation can be cached
/// and reopened instead of rebuilt. The normalized rootfs is deliberately not retained: only
/// its two reported facts are, and those are recorded rather than recomputed.
pub struct Compiled {
    pub store: PathBuf,
    pub(crate) id: CandidateId,
    pub(crate) manifest: GenerationManifest,
    pub tree_digest: String,
    pub entry_count: u32,
}

impl Compiled {
    /// The Candidate identity, derived from the exact published manifest bytes.
    pub fn id(&self) -> &CandidateId {
        &self.id
    }

    /// The published Candidate manifest.
    pub fn manifest(&self) -> &GenerationManifest {
        &self.manifest
    }
}

/// The 1 vCPU Machine shape a test Generation targets.
#[derive(Clone, Copy)]
pub struct Shape {
    pub memory_mib: u64,
    pub storage_mib: u64,
}

/// Returns the compiled Generation for these inputs, building it only on a cache miss.
///
/// `scratch` is retained in the signature because callers still use it for their own artifacts;
/// the compiled store lives in the shared cache, not under it, because it is identical for
/// identical inputs and costs minutes to rebuild.
pub fn compile(
    layout: &Path,
    reference: &str,
    shape: Shape,
    inputs: &Inputs,
    _scratch: &Path,
) -> Compiled {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("generation-cache");
    crate::x86_64_sandbox_boot_generation_cache::compile(&root, layout, reference, shape, inputs)
}

/// Imports, normalizes, and compiles one image for `shape` with one writable class.
pub(crate) fn compile_uncached(
    layout: &Path,
    reference: &str,
    shape: Shape,
    inputs: &Inputs,
    scratch: &Path,
) -> Compiled {
    let Shape {
        memory_mib,
        storage_mib,
    } = shape;
    let store = scratch.join("store");
    fs::create_dir_all(&store).unwrap();
    let platform = OciPlatform::new("linux", "amd64", None).unwrap();
    let imported = import_oci_layout(ImportOciLayout::new(
        layout,
        &store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .expect("import the OCI layout");
    let normalized = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &store,
        RootfsLimits::default(),
    ))
    .expect("normalize the rootfs");
    let workload = normalized.workload();
    let template = TemplateRevision::new(
        TemplateImage::new(
            OciImage::parse(reference).unwrap(),
            workload.manifest_digest().clone(),
            workload.platform().clone(),
        ),
        MachineShape::new(1, memory_mib, storage_mib).unwrap(),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(3600).unwrap(),
        1,
    )
    .unwrap();
    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![storage_mib * MIB];
    let staging = scratch.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let generation = compile_generation(CompileGeneration::new(
        &template,
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
    ))
    .expect("compile the Generation");
    Compiled {
        store,
        id: generation.candidate.id,
        manifest: generation.candidate.manifest,
        tree_digest: normalized.tree_manifest_digest().as_str().to_owned(),
        entry_count: normalized.entry_count(),
    }
}

/// Lowercase hex SHA-256 of a whole file, read from the start; the cursor is rewound after.
pub fn sha256_file(mut file: &File) -> String {
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    file.seek(SeekFrom::Start(0)).unwrap();
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            write!(hex, "{byte:02x}").unwrap();
            hex
        })
}

/// Creates the Instance-private overlay head as a fresh copy of the sterile template.
pub fn private_head(template: &mut File, path: &Path) -> File {
    let _ = fs::remove_file(path);
    let mut head = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    template.seek(SeekFrom::Start(0)).unwrap();
    std::io::copy(template, &mut head).unwrap();
    head.seek(SeekFrom::Start(0)).unwrap();
    head
}
