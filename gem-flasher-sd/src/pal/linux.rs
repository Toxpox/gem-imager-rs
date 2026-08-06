use crate::{Error, Result, helpers::Eject};

use std::io;
use std::path::{Path, PathBuf};

#[cfg(feature = "udev")]
use std::{
    collections::HashMap,
    os::fd::{FromRawFd, IntoRawFd},
};

#[cfg(feature = "udev")]
pub(crate) fn open(dst: &Path) -> Result<LinuxDrive> {
    async fn unmount_filesystem(object: &udisks2::Object) -> anyhow::Result<()> {
        let Ok(filesystem) = object.filesystem().await else {
            return Ok(());
        };
        if filesystem.mount_points().await?.is_empty() {
            return Ok(());
        }

        tracing::info!("Unmounting Linux filesystem {}", object.object_path());
        filesystem.unmount(HashMap::new()).await?;
        Ok(())
    }

    async fn unmount_existing_layout(
        client: &udisks2::Client,
        object: &udisks2::Object,
    ) -> anyhow::Result<()> {
        unmount_filesystem(object).await?;

        let Ok(table) = object.partition_table().await else {
            return Ok(());
        };
        for partition in table.partitions().await? {
            let partition = client.object(partition)?;
            unmount_filesystem(&partition).await?;
        }
        Ok(())
    }

    async fn open_inner(dst: &Path) -> anyhow::Result<LinuxDrive> {
        let dbus_client = udisks2::Client::new().await?;

        let devs = dbus_client
            .manager()
            .resolve_device(
                HashMap::from([("path", dst.to_str().unwrap().into())]),
                HashMap::new(),
            )
            .await?;

        let block = devs
            .first()
            .ok_or(anyhow::anyhow!("Block device not found",))?
            .to_owned();

        let object = dbus_client.object(block).expect("Unexpected error");
        unmount_existing_layout(&dbus_client, &object).await?;

        let block = object.block().await?;

        let fd = block
            .open_device("rw", HashMap::from([("flags", libc::O_DIRECT.into())]))
            .await?;
        let file =
            unsafe { std::fs::File::from_raw_fd(std::os::fd::OwnedFd::from(fd).into_raw_fd()) };

        Ok(LinuxDrive {
            file,
            drive: dst.to_path_buf(),
        })
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    rt.block_on(async move { open_inner(dst).await })
        .map_err(|e| Error::FailedToOpenDestination { source: e })
}

#[cfg(not(feature = "udev"))]
pub(crate) fn open(dst: &Path) -> Result<LinuxDrive> {
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(device) = gem_drivelist::drive_list().ok().and_then(|devices| {
        devices
            .into_iter()
            .find(|device| Path::new(&device.raw) == dst)
    }) {
        for mount in device
            .mountpoints
            .iter()
            .rev()
            .filter(|mount| !mount.path.is_empty())
        {
            tracing::info!("Unmounting Linux mount point {}", mount.path);
            let output = std::process::Command::new("umount")
                .arg(&mount.path)
                .output()
                .map_err(|error| Error::FailedToOpenDestination {
                    source: error.into(),
                })?;
            if !output.status.success() {
                let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(Error::FailedToOpenDestination {
                    source: io::Error::other(if message.is_empty() {
                        format!("umount failed with {}", output.status)
                    } else {
                        message
                    })
                    .into(),
                });
            }
        }
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .custom_flags(libc::O_DIRECT)
        .open(dst)?;

    Ok(LinuxDrive {
        file,
        drive: dst.to_path_buf(),
    })
}

pub(crate) fn format(dst: &Path) -> Result<()> {
    let disk_size = crate::helpers::destination_size(dst)?;
    let drive = open(dst)?;
    crate::helpers::format_device(drive, disk_size)
}

#[derive(Debug)]
pub(crate) struct LinuxDrive {
    file: std::fs::File,
    drive: PathBuf,
}

impl crate::helpers::Commit for LinuxDrive {
    fn commit(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.file)?;
        self.file.sync_all()
    }
}

#[cfg(feature = "udev")]
impl Eject for LinuxDrive {
    fn eject(self) -> io::Result<()> {
        async fn inner(dst: PathBuf) -> io::Result<()> {
            let dbus_client = udisks2::Client::new().await.map_err(io::Error::other)?;

            let devs = dbus_client
                .manager()
                .resolve_device(
                    HashMap::from([("path", dst.to_str().unwrap().into())]),
                    HashMap::new(),
                )
                .await
                .map_err(io::Error::other)?;

            let obj_path = devs
                .first()
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Block device not found",
                ))?
                .to_owned();

            let block = dbus_client
                .object(obj_path)
                .expect("Unexpected error")
                .block()
                .await
                .map_err(io::Error::other)?;

            dbus_client
                .object(block.drive().await.map_err(io::Error::other)?)
                .expect("Unexpected error")
                .drive()
                .await
                .map_err(io::Error::other)?
                .eject(HashMap::new())
                .await
                .map_err(io::Error::other)?;

            Ok(())
        }

        let _ = self.file.sync_all();
        let dst = self.drive.clone();

        std::mem::drop(self);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        rt.block_on(async move { inner(dst).await })
            .map_err(io::Error::other)
    }
}

#[cfg(not(feature = "udev"))]
impl Eject for LinuxDrive {
    fn eject(self) -> std::io::Result<()> {
        let _ = self.file.sync_all();
        let drive = self.drive.clone();
        std::mem::drop(self);

        let output = std::process::Command::new("eject").arg(drive).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8(output.stderr).unwrap(),
            ))
        }
    }
}

impl io::Read for LinuxDrive {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl io::Seek for LinuxDrive {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl io::Write for LinuxDrive {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
