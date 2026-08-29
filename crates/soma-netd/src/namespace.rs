//! Network namespace creation, entry, and destruction with direct syscalls.
//!
//! A namespace is created by `unshare(CLONE_NEWNET)` on a dedicated thread and pinned by
//! bind-mounting that thread's `/proc/thread-self/ns/net` onto one file under the broker's
//! namespace directory, so it outlives the thread and survives a broker restart for
//! reconciliation.
//! Work inside the namespace runs on a scoped thread that calls `setns`, so the broker's
//! main threads never change namespace.

#![allow(unsafe_code)]

use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    thread,
};

use crate::{Error, Step};

const THREAD_NS: &str = "/proc/thread-self/ns/net";

/// One pinned network namespace.
#[derive(Debug)]
pub struct NetNamespace {
    fd: OwnedFd,
    path: PathBuf,
}

impl NetNamespace {
    /// Creates and pins a fresh namespace at `dir/name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingPrivilege`] on `EPERM`, [`Error::InvalidState`] when the pin
    /// file already exists, or [`Error::Kernel`] at the failing step.
    pub fn create(dir: &Path, name: &str) -> Result<Self, Error> {
        fs::create_dir_all(dir).map_err(|error| Error::io(Step::MountNamespace, &error))?;
        let path = dir.join(name);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    Error::InvalidState("namespace pin exists")
                } else {
                    Error::io(Step::MountNamespace, &error)
                }
            })?;
        let target = path.clone();
        let result = thread::Builder::new()
            .name("soma-netd-unshare".to_owned())
            .spawn(move || unshare_and_pin(&target))
            .map_err(|error| Error::io(Step::Thread, &error))?
            .join()
            .map_err(|_| Error::Kernel {
                step: Step::Thread,
                errno: 0,
            })?;
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Self::open(&path)
    }

    /// Opens an existing pinned namespace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::OpenNamespace`].
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = File::open(path).map_err(|error| Error::io(Step::OpenNamespace, &error))?;
        Ok(Self {
            fd: file.into(),
            path: path.to_path_buf(),
        })
    }

    /// Returns the pin path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the namespace descriptor for netlink `IFLA_NET_NS_FD` use.
    #[must_use]
    pub fn as_fd(&self) -> &OwnedFd {
        &self.fd
    }

    /// Runs `work` on a scoped thread that has entered this namespace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::EnterNamespace`] when `setns` fails, or the
    /// error returned by `work`.
    pub fn within<T, F>(&self, work: F) -> Result<T, Error>
    where
        T: Send,
        F: FnOnce() -> Result<T, Error> + Send,
    {
        let raw = self.fd.as_raw_fd();
        thread::scope(|scope| {
            let handle = thread::Builder::new()
                .name("soma-netd-ns".to_owned())
                .spawn_scoped(scope, move || {
                    // SAFETY: `setns` only changes the calling thread's namespace; the
                    // descriptor is owned by `self`, which outlives the scoped thread.
                    if unsafe { libc::setns(raw, libc::CLONE_NEWNET) } != 0 {
                        return Err(Error::kernel(Step::EnterNamespace));
                    }
                    work()
                })
                .map_err(|error| Error::io(Step::Thread, &error))?;
            handle.join().map_err(|_| Error::Kernel {
                step: Step::Thread,
                errno: 0,
            })?
        })
    }

    /// Unpins one namespace file; absent files are not an error.
    ///
    /// The kernel destroys the namespace once no thread, descriptor, mount, or device
    /// still references it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::Unmount`] or [`Step::Unlink`].
    pub fn unpin(path: &Path) -> Result<Unpinned, Error> {
        if !path.exists() {
            return Ok(Unpinned::AlreadyAbsent);
        }
        let target = c_path(path)?;
        // SAFETY: `target` is a valid NUL-terminated path and `MNT_DETACH` has no memory
        // preconditions.
        let result = unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
        if result != 0 {
            let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno != libc::EINVAL && errno != libc::ENOENT {
                return Err(Error::Kernel {
                    step: Step::Unmount,
                    errno,
                });
            }
        }
        match fs::remove_file(path) {
            Ok(()) => Ok(Unpinned::Removed),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Unpinned::AlreadyAbsent),
            Err(error) => Err(Error::io(Step::Unlink, &error)),
        }
    }

    /// Lists the pin names under one directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::OpenNamespace`].
    pub fn list(dir: &Path) -> Result<Vec<String>, Error> {
        let mut names = Vec::new();
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(|error| Error::io(Step::OpenNamespace, &error))?;
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(Step::OpenNamespace, &error)),
        }
        names.sort_unstable();
        Ok(names)
    }

    /// Probes whether the caller may create namespaces; the probe namespace is discarded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingPrivilege`] without `CAP_NET_ADMIN` and `CAP_SYS_ADMIN`.
    pub fn probe_privilege() -> Result<(), Error> {
        thread::Builder::new()
            .name("soma-netd-probe".to_owned())
            .spawn(|| {
                // SAFETY: `unshare` has no memory preconditions and affects only this thread.
                if unsafe { libc::unshare(libc::CLONE_NEWNET) } == 0 {
                    Ok(())
                } else {
                    Err(privilege_error(Step::Unshare))
                }
            })
            .map_err(|error| Error::io(Step::Thread, &error))?
            .join()
            .map_err(|_| Error::Kernel {
                step: Step::Thread,
                errno: 0,
            })?
    }
}

/// The result of unpinning one namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unpinned {
    /// The pin was unmounted and removed now.
    Removed,
    /// No pin existed.
    AlreadyAbsent,
}

fn unshare_and_pin(target: &Path) -> Result<(), Error> {
    // SAFETY: `unshare` has no memory preconditions and affects only this thread.
    if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
        return Err(privilege_error(Step::Unshare));
    }
    let source = CString::new(THREAD_NS).map_err(|_| Error::InvalidState("ns path"))?;
    let target = c_path(target)?;
    let none = c"none";
    // SAFETY: every pointer is a valid NUL-terminated string that outlives the call, the data
    // pointer is null, and `MS_BIND` reads nothing else.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            none.as_ptr(),
            libc::MS_BIND,
            std::ptr::null(),
        )
    };
    if result != 0 {
        return Err(privilege_error(Step::MountNamespace));
    }
    Ok(())
}

fn privilege_error(step: Step) -> Error {
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if errno == libc::EPERM {
        Error::MissingPrivilege("CAP_NET_ADMIN and CAP_SYS_ADMIN")
    } else {
        Error::Kernel { step, errno }
    }
}

fn c_path(path: &Path) -> Result<CString, Error> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| Error::InvalidState("path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_a_missing_directory_is_empty_and_unpin_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("none");
        assert!(NetNamespace::list(&missing).expect("list").is_empty());
        assert_eq!(
            NetNamespace::unpin(&missing.join("x")).expect("absent"),
            Unpinned::AlreadyAbsent
        );
    }

    #[test]
    fn creating_without_privilege_fails_typed_and_leaves_no_pin() {
        if NetNamespace::probe_privilege().is_ok() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let error = NetNamespace::create(dir.path(), "probe").expect_err("unprivileged");
        assert_eq!(
            error,
            Error::MissingPrivilege("CAP_NET_ADMIN and CAP_SYS_ADMIN")
        );
        assert!(NetNamespace::list(dir.path()).expect("list").is_empty());
    }
}
