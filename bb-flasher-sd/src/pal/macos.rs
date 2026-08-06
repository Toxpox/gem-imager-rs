use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub(crate) struct MacOSFile {
    inner: File,
    path: PathBuf,
}

impl Read for MacOSFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for MacOSFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for MacOSFile {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl std::fmt::Debug for MacOSFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacOSFile")
            .field("path", &self.path)
            .field("inner", &self.inner)
            .finish()
    }
}

fn diskutil_device(path: &str) -> String {
    path.strip_prefix("/dev/rdisk")
        .map(|suffix| format!("/dev/disk{suffix}"))
        .unwrap_or_else(|| path.to_owned())
}

fn run_diskutil(action: &str, path: &str) -> std::io::Result<()> {
    let device = diskutil_device(path);
    let output = std::process::Command::new("diskutil")
        .args([action, &device])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(io::Error::other(if message.is_empty() {
            format!("diskutil {action} failed with {}", output.status)
        } else {
            message
        }))
    }
}

fn unmount_disk(path: &str) -> std::io::Result<()> {
    run_diskutil("unmountDisk", path)
}

impl crate::helpers::Commit for MacOSFile {
    fn commit(&mut self) -> std::io::Result<()> {
        self.inner.flush()?;
        self.inner.sync_all()
    }
}

impl crate::helpers::Eject for MacOSFile {
    fn eject(mut self) -> std::io::Result<()> {
        crate::helpers::Commit::commit(&mut self)?;
        let path = self.path.to_string_lossy().into_owned();
        drop(self);
        run_diskutil("eject", &path)
    }
}

pub(crate) fn format(dst: &Path) -> Result<()> {
    let disk_size = crate::helpers::destination_size(dst)?;
    let drive = open(dst)?;
    crate::helpers::format_device(drive, disk_size)
}

fn check_dst(dst: &Path) -> Result<()> {
    match std::fs::exists(dst) {
        Ok(true) => Ok(()),
        _ => Err(Error::FailedToOpenDestination {
            source: anyhow::anyhow!("Drive not found"),
        }),
    }
}

#[cfg(not(feature = "macos_authopen"))]
pub(crate) fn open(dst: &Path) -> Result<MacOSFile> {
    check_dst(dst)?;

    let dst_str = dst.to_string_lossy();
    unmount_disk(&dst_str).map_err(|error| Error::FailedToOpenDestination {
        source: error.into(),
    })?;

    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(dst)
        .map_err(|e| Error::FailedToOpenDestination { source: e.into() })?;

    Ok(MacOSFile {
        inner: f,
        path: dst.to_path_buf(),
    })
}

#[cfg(feature = "macos_authopen")]
pub(crate) fn open(dst: &Path) -> Result<MacOSFile> {
    fn inner(dst: PathBuf) -> anyhow::Result<File> {
        use nix::cmsg_space;
        use nix::sys::socket::{ControlMessageOwned, MsgFlags};
        use security_framework::authorization::{
            Authorization, AuthorizationItemSetBuilder, Flags,
        };
        use std::{
            io::{IoSliceMut, Write},
            os::{
                fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
                unix::net::UnixStream,
            },
            process::{Command, Stdio},
        };

        let rights = AuthorizationItemSetBuilder::new()
            .add_right(format!("sys.openfile.readwrite.{}", dst.to_str().unwrap()))
            .expect("Failed to create right")
            .build();

        let auth = Authorization::new(
            Some(rights),
            None,
            Flags::INTERACTION_ALLOWED | Flags::EXTEND_RIGHTS | Flags::PREAUTHORIZE,
        )
        .expect("Failed to create authorization");

        let form = auth
            .make_external_form()
            .expect("Failed to make external form");
        let (pipe0, pipe1) = UnixStream::pair().expect("Failed to create socket");

        let mut cmd = Command::new("/usr/libexec/authopen")
            .args(["-stdoutpipe", "-extauth", "-o", "2", dst.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(OwnedFd::from(pipe1))
            .spawn()?;

        // Send authorization form
        let mut stdin = cmd.stdin.take().expect("Missing stdin");
        let form_bytes: Vec<u8> = form.bytes.into_iter().map(|x| x as u8).collect();
        stdin
            .write_all(&form_bytes)
            .expect("Failed to write to stdin");
        drop(stdin);

        const IOV_BUF_SIZE: usize =
            unsafe { nix::libc::CMSG_SPACE(std::mem::size_of::<std::ffi::c_int>() as u32) }
                as usize;
        let mut iov_buf = [0u8; IOV_BUF_SIZE];
        let mut iov = [IoSliceMut::new(&mut iov_buf)];

        let mut cmsg = cmsg_space!([RawFd; 1]);

        match nix::sys::socket::recvmsg::<()>(
            pipe0.as_raw_fd(),
            &mut iov,
            Some(&mut cmsg),
            MsgFlags::empty(),
        ) {
            Ok(result) => {
                tracing::info!("Result: {:#?}", result);

                for msg in result.cmsgs().expect("Unexpected error") {
                    if let ControlMessageOwned::ScmRights(scm_rights) = msg {
                        if let Some(fd) = scm_rights.into_iter().next() {
                            tracing::debug!("receive file descriptor");
                            return Ok(unsafe { File::from_raw_fd(fd) });
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Macos Error: {}", e);
            }
        }

        let _ = cmd.wait();

        Err(anyhow::anyhow!("Authopen failed to open the SD Card"))
    }

    check_dst(dst)?;
    let p = dst.to_owned();
    unmount_disk(dst.to_str().unwrap()).map_err(|error| Error::FailedToOpenDestination {
        source: error.into(),
    })?;
    let f = inner(p).map_err(|e| Error::FailedToOpenDestination { source: e })?;

    Ok(MacOSFile {
        inner: f,
        path: dst.to_path_buf(),
    })
}
