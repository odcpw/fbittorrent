use std::{
    collections::VecDeque,
    future::Future,
    io::SeekFrom,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Poll, Waker},
    time::Instant,
};

use anyhow::Context;
use dashmap::DashMap;

use librqbit_core::{
    constants::CHUNK_SIZE,
    lengths::{CurrentPiece, Lengths, ValidPieceIndex},
};
use rqbit_runtime::{
    Cx,
    io::{AsyncRead, AsyncSeek, ReadBuf},
    sync::{AcquireError, OwnedSemaphorePermit},
};
use tracing::{debug, trace};

use crate::{ManagedTorrent, file_info::FileInfo, storage::TorrentStorage};

use super::{ManagedTorrentHandle, TorrentMetadata};

type StreamId = usize;

// 32 mb lookahead by default.
const PER_STREAM_BUF_DEFAULT: u64 = 32 * 1024 * 1024;

struct StreamState {
    file_id: usize,
    file_len: u64,
    file_abs_offset: u64,
    position: u64,
    waker: Option<Waker>,
}

impl StreamState {
    fn current_piece(&self, lengths: &Lengths) -> Option<CurrentPiece> {
        lengths.compute_current_piece(self.position, self.file_abs_offset)
    }

    fn queue<'a>(&self, lengths: &'a Lengths) -> impl Iterator<Item = ValidPieceIndex> + use<'a> {
        let start = self.file_abs_offset + self.position;
        let end = (start + PER_STREAM_BUF_DEFAULT).min(self.file_abs_offset + self.file_len);
        let dpl = lengths.default_piece_length();
        let start_id = (start / dpl as u64).try_into().unwrap();
        let end_id = end.div_ceil(dpl as u64).try_into().unwrap();
        (start_id..end_id).filter_map(|i| lengths.validate_piece_index(i))
    }
}

#[derive(Default)]
pub(crate) struct TorrentStreams {
    next_stream_id: AtomicUsize,
    streams: DashMap<StreamId, StreamState>,
}

impl TorrentStreams {
    fn next_id(&self) -> usize {
        self.next_stream_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register_waker(&self, stream_id: StreamId, waker: Waker) {
        if let Some(mut s) = self.streams.get_mut(&stream_id) {
            let vm = s.value_mut();
            vm.waker = Some(waker);
        }
    }

    // Interleave 1st, 2nd etc pieces from each active stream in turn until they get 1/10th of the file .
    pub(crate) fn iter_next_pieces<'a>(
        &'a self,
        lengths: &'a Lengths,
    ) -> impl Iterator<Item = ValidPieceIndex> + 'a {
        struct Interleave<I> {
            all: VecDeque<I>,
        }

        impl<I: Iterator<Item = ValidPieceIndex>> Iterator for Interleave<I> {
            type Item = ValidPieceIndex;

            fn next(&mut self) -> Option<Self::Item> {
                while let Some(mut it) = self.all.pop_front() {
                    if let Some(piece) = it.next() {
                        self.all.push_back(it);
                        return Some(piece);
                    }
                }
                None
            }
        }

        let mut all: Vec<_> = self.streams.iter().map(|s| s.queue(lengths)).collect();

        // Shuffle to decrease determinism and make queueing fairer.
        use rand::seq::SliceRandom;
        all.shuffle(&mut rand::rng());

        Interleave { all: all.into() }
    }

    pub(crate) fn wake_streams_on_piece_completed(
        &self,
        piece_id: ValidPieceIndex,
        lengths: &Lengths,
    ) {
        let mut wake = Vec::new();
        for mut w in self.streams.iter_mut() {
            if w.value().current_piece(lengths).map(|p| p.id) == Some(piece_id)
                && let Some(waker) = w.value_mut().waker.take()
            {
                debug!(
                    stream_id = *w.key(),
                    piece_id = piece_id.get(),
                    "waking stream"
                );
                wake.push(waker);
            }
        }
        for waker in wake {
            waker.wake();
        }
    }

    fn drop_stream(&self, stream_id: StreamId) -> Option<StreamState> {
        debug!(stream_id, "dropping stream");
        self.streams.remove(&stream_id).map(|s| s.1)
    }

    pub(crate) fn streamed_file_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.streams.iter().map(|s| s.value().file_id)
    }
}

pub struct FileStream {
    cx: Cx,
    torrent: ManagedTorrentHandle,
    metadata: Arc<TorrentMetadata>,
    streams: Arc<TorrentStreams>,
    stream_id: usize,
    file_id: usize,
    position: u64,

    // file params
    file_len: u64,
    file_torrent_abs_offset: u64,

    pending_read_admission: Option<ReadAdmission>,
}

struct ReadAdmission {
    cx: Cx,
    future: Pin<Box<dyn Future<Output = Result<OwnedSemaphorePermit, AcquireError>> + Send>>,
}

macro_rules! map_io_err {
    ($e:expr) => {
        $e.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    };
}

macro_rules! poll_try_io {
    ($e:expr) => {{
        let e = map_io_err!($e);
        match e {
            Ok(r) => r,
            Err(e) => {
                debug!("stream error {e:#}");
                return Poll::Ready(Err(e));
            }
        }
    }};
}

impl AsyncRead for FileStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        tbuf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        poll_try_io!(self.cx.checkpoint());
        if tbuf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        // if the file is over, return 0
        if self.position == self.file_len {
            debug!(
                stream_id = self.stream_id,
                file_id = self.file_id,
                "stream completed, EOF"
            );
            return Poll::Ready(Ok(()));
        }

        let current = poll_try_io!(
            self.metadata
                .lengths()
                .compute_current_piece(self.position, self.file_torrent_abs_offset)
                .context("invalid position")
        );

        // if the piece is not there, register to wake when it is
        // check if we have the piece for real
        let have = poll_try_io!(self.torrent.with_chunk_tracker(|ct| {
            let have = ct.get_have_pieces().as_slice()[current.id.get() as usize];
            if !have {
                self.streams
                    .register_waker(self.stream_id, cx.waker().clone());
            }
            have
        }));
        if !have {
            debug!(stream_id = self.stream_id, file_id = self.file_id, piece_id = %current.id, "poll pending, not have");
            return Poll::Pending;
        }

        // Admit each bounded read in the task that actually polls it. A stream
        // can be returned by its creator, so a lifetime permit would retain an
        // obligation against a task that has already completed.
        let reading_cx = Cx::current().unwrap_or_else(|| self.cx.clone());
        poll_try_io!(reading_cx.checkpoint());
        let mut admission = self
            .pending_read_admission
            .take()
            .filter(|admission| {
                admission.cx.task_id() == reading_cx.task_id()
                    && admission.cx.region_id() == reading_cx.region_id()
            })
            .unwrap_or_else(|| {
                let semaphore = self.torrent.shared.spawner.semaphore();
                let cx = reading_cx.clone();
                ReadAdmission {
                    cx: reading_cx,
                    future: Box::pin(async move {
                        OwnedSemaphorePermit::acquire(semaphore, &cx, 1).await
                    }),
                }
            });
        let _read_permit = match admission.future.as_mut().poll(cx) {
            Poll::Pending => {
                self.pending_read_admission = Some(admission);
                return Poll::Pending;
            }
            Poll::Ready(result) => poll_try_io!(result),
        };

        // actually stream the piece
        let buf = tbuf.unfilled();
        let file_remaining = self.file_len - self.position;
        let bytes_to_read: usize = poll_try_io!(
            (buf.len() as u64)
                .min(current.piece_remaining as u64)
                .min(file_remaining)
                .min(CHUNK_SIZE as u64)
                .try_into()
        );

        let buf = &mut buf[..bytes_to_read];

        let start = Instant::now();
        poll_try_io!(poll_try_io!(self.torrent.shared.spawner.block_in_place(
            || {
                self.torrent.with_storage_and_file(
                    self.file_id,
                    |files, _fi| {
                        files.pread_exact(self.file_id, self.position, buf)?;
                        Ok::<_, anyhow::Error>(())
                    },
                    &self.metadata,
                )
            }
        )));

        trace!(
            buflen = buf.len(),
            stream_id = self.stream_id,
            file_id = self.file_id,
            read_time = ?start.elapsed(),
            "will write bytes"
        );

        self.as_mut().advance(bytes_to_read as u64);
        tbuf.advance(bytes_to_read);

        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for FileStream {
    fn poll_seek(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        position: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        poll_try_io!(self.cx.checkpoint());
        let new_pos = poll_try_io!(seek_position(self.position, self.file_len, position));
        self.as_mut().set_position(new_pos);
        debug!(stream_id = self.stream_id, position = self.position, "seek");
        Poll::Ready(Ok(self.position))
    }
}

fn seek_position(current: u64, end: u64, position: SeekFrom) -> std::io::Result<u64> {
    let new_pos = match position {
        SeekFrom::Start(s) => Some(s),
        SeekFrom::End(delta) => end.checked_add_signed(delta),
        SeekFrom::Current(delta) => current.checked_add_signed(delta),
    };
    new_pos
        .filter(|&p| p <= end)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid seek"))
}

impl Drop for FileStream {
    fn drop(&mut self) {
        self.streams.drop_stream(self.stream_id);
    }
}

impl ManagedTorrent {
    fn with_storage_and_file<F, R>(
        &self,
        file_id: usize,
        f: F,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<R>
    where
        F: FnOnce(&dyn TorrentStorage, &FileInfo) -> R,
    {
        self.with_state(|s| {
            let files = match s {
                crate::ManagedTorrentState::Paused(p) => &*p.files,
                crate::ManagedTorrentState::Live(l) => &*l.files,
                s => anyhow::bail!("with_storage_and_file: invalid state: {}", s.name()),
            };
            let fi = metadata.file_infos.get(file_id).context("invalid file")?;
            Ok(f(files, fi))
        })
    }

    fn streams(&self) -> anyhow::Result<Arc<TorrentStreams>> {
        self.with_state(|s| match s {
            crate::ManagedTorrentState::Paused(p) => Ok(p.streams.clone()),
            crate::ManagedTorrentState::Live(l) => Ok(l.streams.clone()),
            s => anyhow::bail!("streams: invalid state {}", s.name()),
        })
    }

    fn maybe_reconnect_needed_peers_for_file(&self, file_id: usize) -> bool {
        // If we have the full file, don't bother.
        if self.is_file_finished(file_id) {
            return false;
        }
        self.with_state(|state| {
            if let crate::ManagedTorrentState::Live(l) = &state {
                l.reconnect_all_not_needed_peers();
            }
        });
        true
    }

    fn is_file_finished(&self, file_id: usize) -> bool {
        let metadata = self.metadata.load();
        let metadata = match metadata.as_ref() {
            Some(r) => r,
            None => return false,
        };
        // TODO: would be nice to remove locking
        self.with_chunk_tracker(|ct| ct.is_file_finished(&metadata.file_infos[file_id]))
            .unwrap_or(false)
    }

    pub async fn stream(self: Arc<Self>, cx: &Cx, file_id: usize) -> anyhow::Result<FileStream> {
        cx.checkpoint()?;
        let metadata = self
            .metadata
            .load_full()
            .context("torrent metadata is not resolved")?;
        let (fd_len, fd_offset) = self.with_storage_and_file(
            file_id,
            |_fd, fi| (fi.len, fi.offset_in_torrent),
            &metadata,
        )?;
        let streams = self.streams()?;
        let s = FileStream {
            cx: cx.clone(),
            stream_id: streams.next_id(),
            streams: streams.clone(),
            file_id,
            position: 0,

            file_len: fd_len,
            file_torrent_abs_offset: fd_offset,
            pending_read_admission: None,
            torrent: self,
            metadata,
        };
        s.torrent.maybe_reconnect_needed_peers_for_file(file_id);
        streams.streams.insert(
            s.stream_id,
            StreamState {
                file_id,
                position: 0,
                waker: None,
                file_len: fd_len,
                file_abs_offset: fd_offset,
            },
        );

        debug!(stream_id = s.stream_id, file_id, "started stream");

        Ok(s)
    }
}

impl FileStream {
    pub fn position(&self) -> u64 {
        self.position
    }

    fn advance(&mut self, diff: u64) {
        self.set_position(self.position + diff)
    }

    fn set_position(&mut self, new_pos: u64) {
        self.position = new_pos;
        self.streams
            .streams
            .get_mut(&self.stream_id)
            .unwrap()
            .value_mut()
            .position = new_pos;
    }

    pub fn len(&self) -> u64 {
        self.file_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
    use rqbit_runtime::{Budget, RuntimeBuilder, io::AsyncReadExt, task};

    #[test]
    fn stream_crosses_creator_task_without_retaining_its_permit() {
        let directory = tempfile::TempDir::new().unwrap();
        let payload = vec![b'x'; CHUNK_SIZE as usize * 2 + 7];
        let path = directory.path().join("fixture.bin");
        std::fs::write(&path, &payload).unwrap();
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        runtime.block_on(async {
            let metadata = crate::create_torrent(
                &cx,
                &path,
                crate::CreateTorrentOptions::default(),
                &crate::spawn_utils::BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap();
            let session = Session::new_with_opts(
                &cx,
                directory.path().to_path_buf(),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let torrent = session
                .add_torrent(
                    &cx,
                    AddTorrent::from_bytes(metadata),
                    Some(AddTorrentOptions {
                        paused: true,
                        overwrite: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap()
                .into_handle()
                .unwrap();
            torrent.wait_until_initialized(&cx).await.unwrap();
            let stream =
                task::spawn_with(&cx, move |cx| async move { torrent.stream(&cx, 0).await })
                    .await
                    .unwrap()
                    .unwrap();
            let bytes = task::spawn_with(&cx, move |_cx| async move {
                let mut stream = stream;
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).await.unwrap();
                bytes
            })
            .await
            .unwrap();
            assert_eq!(bytes, payload);
            session.stop(&cx).await.unwrap();
        });
    }
}
