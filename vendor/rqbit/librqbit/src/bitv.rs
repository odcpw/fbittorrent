use std::path::PathBuf;

use anyhow::Context;
use bitvec::{boxed::BitBox, order::Msb0, slice::BitSlice, vec::BitVec};
use rqbit_runtime::Cx;

use crate::{spawn_utils::BlockingSpawner, storage::filesystem::OurFileExt};

pub trait BitV: Send + Sync {
    fn as_slice(&self) -> &BitSlice<u8, Msb0>;
    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0>;
    fn into_dyn(self) -> Box<dyn BitV>;
    fn as_bytes(&self) -> &[u8];
    fn flush(&mut self, flush_async: bool) -> anyhow::Result<()>;
}

pub type BoxBitV = Box<dyn BitV>;

/// A small in-memory bitfield backed by a positioned, durable disk snapshot.
///
/// Persistence completes before `flush` returns. This synchronous storage
/// contract avoids an unowned background flusher whose final write could be
/// abandoned during session shutdown.
pub struct DiskBackedBitV {
    bv: BitBox<u8, Msb0>,
    file: std::fs::File,
    filename: PathBuf,
}

impl Drop for DiskBackedBitV {
    fn drop(&mut self) {
        if let Err(e) = self.flush(false) {
            tracing::warn!(filename = ?self.filename, "error flushing bitv on drop: {e:#}");
        }
    }
}

impl DiskBackedBitV {
    pub async fn new(cx: &Cx, filename: PathBuf, spawner: BlockingSpawner) -> anyhow::Result<Self> {
        let disk_filename = filename.clone();
        let (buf, file) = spawner
            .run(cx, move || {
                let buf = std::fs::read(&disk_filename)
                    .with_context(|| format!("error reading {disk_filename:?}"))?;
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(false)
                    .open(&disk_filename)
                    .with_context(|| format!("error opening {disk_filename:?}"))?;
                Ok::<_, anyhow::Error>((buf, file))
            })
            .await??;
        Ok(Self {
            bv: BitVec::from_vec(buf).into_boxed_bitslice(),
            file,
            filename,
        })
    }
}

#[async_trait::async_trait]
impl BitV for BitBox<u8, Msb0> {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        self.as_bitslice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        self.as_mut_bitslice()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_raw_slice()
    }

    fn flush(&mut self, _flush_async: bool) -> anyhow::Result<()> {
        Ok(())
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        Box::new(self)
    }
}

impl BitV for DiskBackedBitV {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        self.bv.as_bitslice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        self.bv.as_mut_bitslice()
    }

    fn as_bytes(&self) -> &[u8] {
        self.bv.as_raw_slice()
    }

    fn flush(&mut self, _flush_async: bool) -> anyhow::Result<()> {
        let result = self
            .file
            .pwrite_all(0, self.bv.as_raw_slice())
            .and_then(|()| self.file.sync_all().map_err(Into::into));
        if result.is_err() {
            // Never restore a possibly partial bitfield as a trusted hash check.
            if let Err(e) = std::fs::remove_file(&self.filename) {
                tracing::error!(filename = ?self.filename, "error removing invalid bitv: {e:#}");
            }
        }
        result.with_context(|| format!("error flushing bitv {:?}", self.filename))
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        Box::new(self)
    }
}

impl BitV for Box<dyn BitV> {
    fn as_slice(&self) -> &BitSlice<u8, Msb0> {
        (**self).as_slice()
    }

    fn as_slice_mut(&mut self) -> &mut BitSlice<u8, Msb0> {
        (**self).as_slice_mut()
    }

    fn as_bytes(&self) -> &[u8] {
        (**self).as_bytes()
    }

    fn flush(&mut self, flush_async: bool) -> anyhow::Result<()> {
        (**self).flush(flush_async)
    }

    fn into_dyn(self) -> Box<dyn BitV> {
        self
    }
}

#[cfg(test)]
mod tests {
    use rqbit_runtime::{Budget, RuntimeBuilder};

    use super::{BitV, DiskBackedBitV};
    use crate::spawn_utils::BlockingSpawner;

    #[test]
    fn disk_bitfield_persists_flush_and_final_drop() {
        let dir = tempfile::TempDir::new().unwrap();
        let filename = dir.path().join("pieces");
        std::fs::write(&filename, [0, 0]).unwrap();
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let mut bitv = runtime
            .block_on(DiskBackedBitV::new(
                &cx,
                filename.clone(),
                BlockingSpawner::new(1),
            ))
            .unwrap();
        bitv.as_slice_mut().set(0, true);
        bitv.as_slice_mut().set(9, true);
        bitv.flush(true).unwrap();
        assert_eq!(std::fs::read(&filename).unwrap(), [0x80, 0x40]);
        bitv.as_slice_mut().set(0, false);
        drop(bitv);
        assert_eq!(std::fs::read(&filename).unwrap(), [0x00, 0x40]);
        let restored = runtime
            .block_on(DiskBackedBitV::new(&cx, filename, BlockingSpawner::new(1)))
            .unwrap();
        assert_eq!(restored.as_bytes(), [0x00, 0x40]);
    }
}
