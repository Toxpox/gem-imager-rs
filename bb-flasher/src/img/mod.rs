//! Module to handle extraction of compressed firmware, auto detection of type of extraction, etc

#[cfg(feature = "piped_image")]
use bb_helper::file_stream::ReaderFileStream;
use rc_zip_sync::ReadZipStreaming;
use std::{
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};
#[cfg(feature = "piped_image")]
use tokio_util::task::AbortOnDropHandle;

#[cfg(test)]
mod test;

const XZ_MAGIC: [u8; 6] = [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00];

pub struct OsImage {
    size: u64,
    img: OsImageCompression<OsImageSource>,
}

impl OsImage {
    pub fn from_path(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut img = OsImageCompression::new(OsImageSource::from(file))?;

        let size = match &mut img {
            OsImageCompression::Xz(x) => {
                let size = liblzma::uncompressed_size(x.get_mut())?;
                x.get_mut().rewind()?;
                size
            }
            OsImageCompression::Zip(x) => x.entry().uncompressed_size,
            OsImageCompression::Uncompressed(x) => match x.get_ref() {
                OsImageSource::File(file) => file.metadata()?.len(),
                #[cfg(feature = "piped_image")]
                OsImageSource::FileStream { .. } => unreachable!(),
            },
            OsImageCompression::QCow2(x) => x.virtual_disk_size(),
        };

        Ok(Self { size, img })
    }

    #[cfg(feature = "piped_image")]
    pub fn from_piped(
        img: ReaderFileStream,
        _background: AbortOnDropHandle<io::Result<()>>,
        size: u64,
    ) -> io::Result<Self> {
        Ok(Self {
            size,
            img: OsImageCompression::new(OsImageSource::FileStream {
                reader: img,
                _background,
            })?,
        })
    }

    pub(crate) const fn size(&self) -> u64 {
        self.size
    }
}

impl Read for OsImage {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.img {
            OsImageCompression::Xz(x) => x.read(buf),
            OsImageCompression::Zip(x) => x.read(buf),
            OsImageCompression::Uncompressed(x) => x.read(buf),
            OsImageCompression::QCow2(x) => x.read(buf),
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum OsImageCompression<I: Read + Seek> {
    Xz(liblzma::read::XzDecoder<I>),
    Zip(rc_zip_sync::StreamingEntryReader<I>),
    QCow2(qcow2::Qcow2Reader<I>),
    Uncompressed(io::BufReader<I>),
}

impl<I: Read + Seek> OsImageCompression<I> {
    fn new(mut img: I) -> io::Result<Self> {
        let mut magic = [0u8; 6];
        img.read_exact(&mut magic)?;
        img.rewind()?;

        match magic {
            XZ_MAGIC => Ok(Self::Xz(liblzma_new(img))),
            [0x51, 0x46, 0x49, _, _, _] => {
                tracing::info!("Detected qcow2 image");
                qcow2::Qcow2Reader::from_reader(img)
                    .map_err(io::Error::other)
                    .map(Self::QCow2)
            }
            [0x50, 0x4b, 0x03, 0x04, _, _] => img
                .stream_zip_entries_throwing_caution_to_the_wind()
                .map(Self::Zip)
                .map_err(Into::into),
            _ => Ok(Self::Uncompressed(std::io::BufReader::new(img))),
        }
    }
}

impl<I: Read + Seek> Read for OsImageCompression<I> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            OsImageCompression::Xz(x) => x.read(buf),
            OsImageCompression::Zip(x) => x.read(buf),
            OsImageCompression::Uncompressed(x) => x.read(buf),
            OsImageCompression::QCow2(x) => x.read(buf),
        }
    }
}

enum OsImageSource {
    File(std::fs::File),
    #[cfg(feature = "piped_image")]
    FileStream {
        reader: ReaderFileStream,
        _background: AbortOnDropHandle<io::Result<()>>,
    },
}

impl From<std::fs::File> for OsImageSource {
    fn from(value: std::fs::File) -> Self {
        Self::File(value)
    }
}

impl Read for OsImageSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            OsImageSource::File(x) => x.read(buf),
            #[cfg(feature = "piped_image")]
            OsImageSource::FileStream { reader, .. } => reader.read(buf),
        }
    }
}

impl Seek for OsImageSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            OsImageSource::File(file) => file.seek(pos),
            #[cfg(feature = "piped_image")]
            OsImageSource::FileStream { reader, .. } => reader.seek(pos),
        }
    }
}

fn liblzma_new<R: io::Read>(r: R) -> liblzma::read::XzDecoder<R> {
    #[cfg(target_arch = "wasm32")]
    return liblzma::read::XzDecoder::new(r);
    #[cfg(not(target_arch = "wasm32"))]
    liblzma::read::XzDecoder::new_parallel(r)
}
