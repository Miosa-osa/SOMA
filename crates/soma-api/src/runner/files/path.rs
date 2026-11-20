//! Which guest paths the platform's file contract reaches, and the one form each is sent in.
//!
//! The rule is the platform's own (`Web.Controllers.SandboxFiles.PathPolicy`), restated here so a
//! sandbox on this runner admits exactly the paths a sandbox on the platform admits: a relative
//! path resolves from the guest home, a `..` segment or a nul byte is refused outright, and what
//! remains must sit under one of the exposed trees and outside the system ones.
//!
//! The check is on the text of the path, as it is on the platform. The sandbox boundary is the
//! virtual machine: a caller who can run commands in it can already read anything a symbolic
//! link inside it could point at, so following links here would add a second policy without
//! adding a boundary. What this does guarantee is that the bytes the guest is handed name the
//! path the policy approved, absolute and with no segment left to reinterpret.

use core::fmt;

/// Where a relative path starts, as on the platform.
pub(crate) const GUEST_HOME: &str = "/home/user";

/// The trees the contract exposes.
const ALLOWED_ROOTS: [&str; 7] = [
    "/home",
    "/workspace",
    "/tmp",
    "/var/log",
    "/var/lib",
    "/opt",
    "/usr/local",
];

/// The trees the contract never exposes, whatever the allowed list says.
const FORBIDDEN_ROOTS: [&str; 4] = ["/etc", "/proc", "/sys", "/dev"];

/// A path the contract refused before anything reached the guest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PathRefused;

/// One admitted guest path, absolute and normalized.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct GuestPath(String);

impl GuestPath {
    /// Admits `raw` when the contract exposes it, and returns the form the guest is sent.
    ///
    /// # Errors
    ///
    /// Returns [`PathRefused`] for an empty path, a nul byte, a `..` segment, a path that starts
    /// at a home directory the caller cannot see (`~`), a path outside every exposed tree, or one
    /// longer than the guest protocol carries.
    pub(crate) fn admit(raw: &str) -> Result<Self, PathRefused> {
        if raw.is_empty() || raw.contains('\0') || raw.split('/').any(|segment| segment == "..") {
            return Err(PathRefused);
        }
        // The platform expands `~` to the home of the process serving the request, which is
        // outside every exposed tree, so such a path is refused there; it is refused here by name.
        if raw == "~" || raw.starts_with("~/") {
            return Err(PathRefused);
        }
        let normalized = normalize(raw);
        let exposed = ALLOWED_ROOTS.iter().any(|root| under(&normalized, root))
            && !FORBIDDEN_ROOTS.iter().any(|root| under(&normalized, root));
        if !exposed || soma::check_guest_path(normalized.as_bytes()).is_err() {
            return Err(PathRefused);
        }
        Ok(Self(normalized))
    }

    /// A path as soma-api's own `/filesystem` routes carry it: the guest protocol's shape rules
    /// and nothing else, because on that surface what is admissible is the guest's decision
    /// (`soma::check_guest_path`). The platform's exposed trees do not apply there.
    ///
    /// # Errors
    ///
    /// Returns [`PathRefused`] for a path the guest protocol will not carry.
    pub(crate) fn carried(raw: &str) -> Result<Self, PathRefused> {
        soma::check_guest_path(raw.as_bytes()).map_err(|_| PathRefused)?;
        Ok(Self(raw.to_owned()))
    }

    /// The path as the guest is sent it.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The last segment, empty for the root.
    pub(crate) fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }

    /// The directory holding this path, as the guest is sent it.
    ///
    /// It is not re-admitted: the parent of an exposed path is an exposed tree or one of the
    /// directories above the trees, and a caller asking for the directory to exist is asking for
    /// nothing a write into the exposed path would not need.
    pub(crate) fn parent(&self) -> Self {
        match self.0.rfind('/') {
            Some(0) | None => Self("/".to_owned()),
            Some(index) => Self(self.0[..index].to_owned()),
        }
    }

    /// A path named `name` beside this one, for a temporary file the runner names itself.
    ///
    /// It is not re-admitted: `name` is the runner's own and carries no separator, and the
    /// directory is the one the admitted path already names.
    pub(crate) fn sibling(&self, name: &str) -> Self {
        let parent = self.parent();
        let separator = if parent.0 == "/" { "" } else { "/" };
        Self(format!("{}{separator}{name}", parent.0))
    }

    /// A path named `name` inside this one, admitted like any other.
    ///
    /// # Errors
    ///
    /// Returns [`PathRefused`] when the joined path is not one the contract exposes.
    pub(crate) fn join(&self, name: &str) -> Result<Self, PathRefused> {
        if name.is_empty() || name.contains('/') {
            return Err(PathRefused);
        }
        Self::admit(&format!("{}/{name}", self.0.trim_end_matches('/')))
    }
}

impl fmt::Debug for GuestPath {
    /// A guest path is tenant data, so only its length reaches a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GuestPath({} bytes)", self.0.len())
    }
}

/// The absolute form of `raw`, with empty and `.` segments removed.
///
/// `..` never reaches here, so no segment needs to be resolved against the one before it.
fn normalize(raw: &str) -> String {
    let joined = if raw.starts_with('/') {
        raw.to_owned()
    } else {
        format!("{GUEST_HOME}/{raw}")
    };
    let segments: Vec<&str> = joined
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    format!("/{}", segments.join("/"))
}

fn under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::{GuestPath, PathRefused};

    fn admitted(raw: &str) -> String {
        GuestPath::admit(raw)
            .unwrap_or_else(|_| panic!("{raw} should be admitted"))
            .as_str()
            .to_owned()
    }

    #[test]
    fn a_relative_path_starts_at_the_guest_home() {
        assert_eq!(admitted("app.js"), "/home/user/app.js");
        assert_eq!(admitted("./src/./main.rs"), "/home/user/src/main.rs");
    }

    #[test]
    fn repeated_and_trailing_separators_are_folded() {
        assert_eq!(admitted("//workspace///a//b/"), "/workspace/a/b");
        assert_eq!(admitted("/tmp"), "/tmp");
    }

    #[test]
    fn every_exposed_tree_is_admitted() {
        for root in [
            "/home/x",
            "/workspace/x",
            "/tmp/x",
            "/var/log/x",
            "/var/lib/x",
            "/opt/x",
            "/usr/local/x",
        ] {
            assert_eq!(admitted(root), root);
        }
    }

    #[test]
    fn paths_outside_the_exposed_trees_are_refused() {
        for raw in [
            "/",
            "/etc/passwd",
            "/proc/1/environ",
            "/sys",
            "/dev/kvm",
            "/root",
            "/usr/bin",
            "/var",
            "/homework",
            "/tmpfoo",
            "/var/logs",
        ] {
            assert_eq!(GuestPath::admit(raw), Err(PathRefused), "{raw}");
        }
    }

    #[test]
    fn traversal_nul_home_and_empty_paths_are_refused() {
        for raw in [
            "",
            "..",
            "/tmp/../etc/passwd",
            "a/../../etc",
            "/tmp/a\0b",
            "~",
            "~/x",
        ] {
            assert_eq!(GuestPath::admit(raw), Err(PathRefused), "{raw:?}");
        }
        // A name that merely contains dots or a tilde is a name.
        assert_eq!(admitted("/tmp/..hidden"), "/tmp/..hidden");
        assert_eq!(admitted("~notes"), "/home/user/~notes");
    }

    #[test]
    fn a_path_longer_than_the_guest_protocol_carries_is_refused() {
        let long = format!("/tmp/{}", "a".repeat(soma::MAX_GUEST_PATH_BYTES));
        assert_eq!(GuestPath::admit(&long), Err(PathRefused));
    }

    #[test]
    fn the_parent_name_and_join_follow_the_normalized_path() {
        let path = GuestPath::admit("/workspace/dir/file.txt").expect("admitted");
        assert_eq!(path.file_name(), "file.txt");
        assert_eq!(path.parent().as_str(), "/workspace/dir");
        assert_eq!(
            GuestPath::admit("/tmp")
                .expect("admitted")
                .parent()
                .as_str(),
            "/"
        );
        let directory = GuestPath::admit("/workspace/dir").expect("admitted");
        assert_eq!(
            directory.join("file.txt").expect("joined").as_str(),
            "/workspace/dir/file.txt"
        );
        assert_eq!(directory.join("a/b"), Err(PathRefused));
        assert_eq!(directory.join(".."), Err(PathRefused));
    }

    #[test]
    fn a_debug_rendering_never_shows_the_path() {
        let path = GuestPath::admit("/tmp/secret-name").expect("admitted");
        assert_eq!(format!("{path:?}"), "GuestPath(16 bytes)");
    }
}
