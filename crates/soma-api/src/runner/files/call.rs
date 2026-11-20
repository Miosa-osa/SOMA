//! Which file call one request names.
//!
//! The platform serves its sandbox file routes under two prefixes, `fs` and the older `files`,
//! with the same actions behind both. Every route either prefix names is resolved here, the ones
//! this runner does not serve included, so such a call is told it is not served rather than
//! hunting for a path that exists on the platform.

/// One file call on one sandbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileCall {
    /// `GET fs`, `GET files`, `GET files/list`: one directory's entries.
    List,
    /// `GET fs/read`: a file's contents, or part of them.
    Read,
    /// `POST fs/write`: a file's contents from a JSON body.
    Write,
    /// `POST fs/stat`: what one path is.
    Stat,
    /// `POST fs/mkdir`: a directory and its missing parents.
    MakeDirectory,
    /// `POST fs/rename`: a move.
    Rename,
    /// `DELETE fs`, `DELETE files`: a removal.
    Delete,
    /// `POST fs/chmod`: permission bits.
    Chmod,
    /// A route of the platform's contract this runner does not serve, by its platform name.
    Unserved(&'static str),
}

impl FileCall {
    /// The operation's name, as a refusal names it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read => "read",
            Self::Write => "write",
            Self::Stat => "stat",
            Self::MakeDirectory => "mkdir",
            Self::Rename => "rename",
            Self::Delete => "delete",
            Self::Chmod => "chmod",
            Self::Unserved(route) => route,
        }
    }

    /// Resolves the segments after `<id>/fs` or `<id>/files`, or `None` for no platform route.
    pub(crate) fn resolve(method: &http::Method, prefix: &str, rest: &[&str]) -> Option<Self> {
        let files = prefix == "files";
        let get = *method == http::Method::GET;
        let post = *method == http::Method::POST;
        Some(match rest {
            [] if get => Self::List,
            [] if *method == http::Method::DELETE => Self::Delete,
            [] if post && files => Self::Unserved("legacy upload"),
            ["list"] if get && files => Self::List,
            ["read"] if get => Self::Read,
            ["write"] if post => Self::Write,
            ["stat"] if post => Self::Stat,
            ["mkdir"] if post => Self::MakeDirectory,
            ["rename"] if post => Self::Rename,
            ["chmod"] if post => Self::Chmod,
            ["upload"] if post => Self::Unserved("upload"),
            ["download"] if get => Self::Unserved("download"),
            ["write-batch" | "write-many"] if post => Self::Unserved("write-many"),
            ["copy"] if post => Self::Unserved("copy"),
            ["tree"] if get && files => Self::Unserved("tree"),
            ["watch"] if get && files => Self::Unserved("watch"),
            ["transfer-url"] if post && files => Self::Unserved("transfer-url"),
            [_, ..] if get && files => Self::Unserved("legacy download"),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use http::Method;

    use super::FileCall;

    fn resolve(method: Method, prefix: &str, rest: &[&str]) -> Option<FileCall> {
        FileCall::resolve(&method, prefix, rest)
    }

    #[test]
    fn both_prefixes_name_the_same_calls() {
        for prefix in ["fs", "files"] {
            assert_eq!(resolve(Method::GET, prefix, &[]), Some(FileCall::List));
            assert_eq!(resolve(Method::DELETE, prefix, &[]), Some(FileCall::Delete));
            assert_eq!(
                resolve(Method::GET, prefix, &["read"]),
                Some(FileCall::Read)
            );
            assert_eq!(
                resolve(Method::POST, prefix, &["write"]),
                Some(FileCall::Write)
            );
            assert_eq!(
                resolve(Method::POST, prefix, &["stat"]),
                Some(FileCall::Stat)
            );
            assert_eq!(
                resolve(Method::POST, prefix, &["mkdir"]),
                Some(FileCall::MakeDirectory)
            );
            assert_eq!(
                resolve(Method::POST, prefix, &["rename"]),
                Some(FileCall::Rename)
            );
            assert_eq!(
                resolve(Method::POST, prefix, &["chmod"]),
                Some(FileCall::Chmod)
            );
        }
        assert_eq!(
            resolve(Method::GET, "files", &["list"]),
            Some(FileCall::List)
        );
    }

    #[test]
    fn a_platform_route_this_runner_does_not_serve_is_named() {
        assert_eq!(
            resolve(Method::POST, "fs", &["copy"]),
            Some(FileCall::Unserved("copy"))
        );
        assert_eq!(
            resolve(Method::GET, "files", &["etc", "hosts"]),
            Some(FileCall::Unserved("legacy download"))
        );
    }

    #[test]
    fn a_wrong_method_or_an_unknown_action_is_no_route() {
        assert_eq!(resolve(Method::POST, "fs", &["read"]), None);
        assert_eq!(resolve(Method::GET, "fs", &["write"]), None);
        assert_eq!(resolve(Method::GET, "fs", &["list"]), None);
        assert_eq!(resolve(Method::POST, "fs", &["format"]), None);
        assert_eq!(resolve(Method::PUT, "fs", &[]), None);
    }
}
