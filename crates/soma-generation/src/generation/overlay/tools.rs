//! The pinned e2fsprogs tools of the overlay phase and their exact invocations.
//!
//! Every tool that formats, populates, checks, or inspects a template is opened once, measured
//! through that descriptor, executed through that same descriptor, and bound into the builder
//! environment, so the digest a Generation carries names the executables that actually ran.

use std::{ffi::OsString, fs, path::Path};

use super::{
    super::{
        erofs::format_uuid,
        error::{CompileError, CompilePhase},
        process::{Invocation, PinnedTool, ToolOutcome, tool_path},
        request::CompilerProfile,
        toolchain::{BoundTool, BuilderEnvironment},
    },
    OVERLAY_FEATURES, OVERLAY_VOLUME_LABEL, OverlayClassEvidence, derive_overlay_hash_seed,
    derive_overlay_uuid, io_error, toolchain, verify,
};

/// The four e2fsprogs tools this phase executes, each opened, measured, and held open.
///
/// Every one of them materially shapes or judges an overlay template, so every one is bound
/// into the builder environment rather than only the formatter.
pub(super) struct PinnedTools {
    pub(super) formatter: PinnedTool,
    populator: PinnedTool,
    checker: PinnedTool,
    inspector: PinnedTool,
}

impl PinnedTools {
    pub(super) fn open(directory: &Path) -> Result<Self, CompileError> {
        let phase = CompilePhase::BuildOverlay;
        Ok(Self {
            formatter: PinnedTool::open(&tool_path(directory, "mke2fs"), phase)?,
            populator: PinnedTool::open(&tool_path(directory, "debugfs"), phase)?,
            checker: PinnedTool::open(&tool_path(directory, "e2fsck"), phase)?,
            inspector: PinnedTool::open(&tool_path(directory, "dumpe2fs"), phase)?,
        })
    }

    const fn each(&self) -> [&PinnedTool; 4] {
        [
            &self.formatter,
            &self.populator,
            &self.checker,
            &self.inspector,
        ]
    }

    fn select(&self, tool: OverlayTool) -> &PinnedTool {
        match tool {
            OverlayTool::Formatter => &self.formatter,
            OverlayTool::Populator => &self.populator,
            OverlayTool::Checker => &self.checker,
            OverlayTool::Inspector => &self.inspector,
        }
    }

    /// Binds every pinned tool under the one e2fsprogs revision they share.
    pub(super) fn bind(&self, revision: &str) -> Result<BuilderEnvironment, CompileError> {
        let phase = CompilePhase::BuildOverlay;
        let mut environment = BuilderEnvironment::new();
        for tool in self.each() {
            environment.bind(
                BoundTool::new(tool.name(), tool.digest(), revision, phase)?,
                phase,
            )?;
        }
        Ok(environment)
    }
}

/// Which pinned e2fsprogs tool one invocation runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayTool {
    Formatter,
    Populator,
    Checker,
    Inspector,
}

pub(super) struct Tools<'a> {
    pub(super) pinned: &'a PinnedTools,
    pub(super) environment: Vec<(String, String)>,
    pub(super) staging: &'a Path,
    pub(super) profile: &'a CompilerProfile,
}

impl Tools<'_> {
    fn run(
        &self,
        program: OverlayTool,
        arguments: Vec<OsString>,
        phase: CompilePhase,
    ) -> Result<ToolOutcome, CompileError> {
        Invocation {
            program: self.pinned.select(program),
            arguments,
            environment: self.environment.clone(),
            working_directory: self.staging,
            deadline: self.profile.tool_deadline,
            phase,
        }
        .run()
    }

    pub(super) fn build_class(
        &self,
        capacity: u64,
        image: &Path,
    ) -> Result<OverlayClassEvidence, CompileError> {
        fs::File::create(image)
            .and_then(|file| file.set_len(capacity))
            .map_err(|_| io_error())?;
        let build = CompilePhase::BuildOverlay;
        let check_phase = CompilePhase::VerifyOverlay;
        let format = self.run(
            OverlayTool::Formatter,
            mke2fs_arguments(capacity, image),
            build,
        )?;
        let populate = vec![
            self.run(
                OverlayTool::Populator,
                debugfs(image, true, "mkdir upper"),
                build,
            )?,
            self.run(
                OverlayTool::Populator,
                debugfs(image, true, "mkdir work"),
                build,
            )?,
        ];
        if !format.succeeded() || populate.iter().any(|outcome| !outcome.succeeded()) {
            return Err(toolchain(build));
        }
        let check = self.run(
            OverlayTool::Checker,
            vec!["-fn".into(), image.into()],
            check_phase,
        )?;
        let inspect = vec![
            self.run(
                OverlayTool::Inspector,
                vec!["-h".into(), image.into()],
                check_phase,
            )?,
            self.run(
                OverlayTool::Populator,
                debugfs(image, false, "ls -l /"),
                check_phase,
            )?,
            self.run(
                OverlayTool::Populator,
                debugfs(image, false, "ls -l /upper"),
                check_phase,
            )?,
            self.run(
                OverlayTool::Populator,
                debugfs(image, false, "ls -l /work"),
                check_phase,
            )?,
        ];
        verify::verify_class(capacity, &check, &inspect)?;
        Ok(OverlayClassEvidence {
            capacity,
            format,
            populate,
            check,
            inspect,
        })
    }
}

fn mke2fs_arguments(capacity: u64, image: &Path) -> Vec<OsString> {
    let extended = format!(
        "hash_seed={},lazy_itable_init=0,lazy_journal_init=0,root_owner=0:0",
        format_uuid(&derive_overlay_hash_seed(capacity))
    );
    [
        "-F", "-q", "-t", "ext4", "-b", "4096", "-I", "256", "-m", "0", "-U",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([
        OsString::from(format_uuid(&derive_overlay_uuid(capacity))),
        "-L".into(),
        OVERLAY_VOLUME_LABEL.into(),
        "-E".into(),
        extended.into(),
        "-O".into(),
        OVERLAY_FEATURES.join(",").into(),
        image.as_os_str().to_owned(),
    ])
    .collect()
}

fn debugfs(image: &Path, write: bool, request: &str) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if write {
        arguments.push(OsString::from("-w"));
    }
    arguments.push("-R".into());
    arguments.push(request.into());
    arguments.push(image.as_os_str().to_owned());
    arguments
}
