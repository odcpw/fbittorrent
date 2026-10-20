//! Verified, seek-aware reads. Sparse payload files are never playback streams.
use super::*;
use rqbit_runtime::io::{AsyncReadExt, AsyncSeekExt};
use std::io::{SeekFrom, Write};

#[derive(Clone)]
pub(crate) struct Video {
    pub file: usize,
    pub len: u64,
    pub mime: &'static str,
}

fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "ts" => "video/mp2t",
        "mp4" | "m4v" => "video/mp4",
        _ => "video/x-matroska",
    }
}

impl EngineDispatcher {
    pub(crate) fn incomplete_video(
        &self,
        hash: &str,
        file: Option<usize>,
    ) -> Result<Option<Video>> {
        let Ok(handle) = self.inner.handle(hash) else {
            return Ok(None);
        };
        let Some(metadata) = handle.metadata.load_full() else {
            return Ok(None);
        };
        let selected = handle.only_files();
        let file = file.or_else(|| {
            crate::daemon::watch::largest_video(
                metadata
                    .file_infos
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| selected.as_ref().is_none_or(|ids| ids.contains(i)))
                    .map(|(i, f)| (i, f.relative_filename.as_path(), f.len)),
            )
        });
        let Some(file) = file else { return Ok(None) };
        let info = metadata
            .file_infos
            .get(file)
            .context("invalid video file")?;
        let complete = handle.have_pieces_snapshot().is_some_and(|bits| {
            info.piece_range
                .clone()
                .all(|i| bits.as_slice().get(i as usize).is_some_and(|bit| *bit))
        });
        if !crate::daemon::watch::video(&info.relative_filename)
            || selected.as_ref().is_some_and(|ids| !ids.contains(&file))
            || complete
        {
            return Ok(None);
        }
        Ok(Some(Video {
            file,
            len: info.len,
            mime: media_type(&info.relative_filename),
        }))
    }

    pub(crate) fn stream_video(
        &self,
        hash: &str,
        video: &Video,
        range: std::ops::Range<u64>,
        output: &mut std::net::TcpStream,
        stopping: &AtomicBool,
    ) -> Result<()> {
        let cx = self.runtime.cx();
        let handle = self.inner.handle(hash)?;
        let mut stream = self.runtime.try_block_on(handle.stream(cx, video.file))??;
        anyhow::ensure!(stream.len() == video.len, "video changed during playback");
        self.runtime
            .try_block_on(stream.seek(SeekFrom::Start(range.start)))??;
        let mut remaining = range.end - range.start;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            cx.checkpoint()?;
            anyhow::ensure!(
                !stopping.load(Ordering::Acquire) && !self.inner.stopping.load(Ordering::Acquire),
                "playback stopped"
            );
            let count = remaining.min(buffer.len() as u64) as usize;
            // Do not retain the runtime's read guard across socket backpressure
            // or an unbounded missing-piece wait. Shutdown can take ownership
            // between these bounded polls, including an engine-slot restart.
            let read = self.runtime.try_block_on(time::timeout(
                cx,
                Duration::from_millis(250),
                stream.read(&mut buffer[..count]),
            ))?;
            let n = match read {
                Ok(result) => result?,
                Err(_) => {
                    let mut byte = [0];
                    output.set_nonblocking(true)?;
                    let alive = match output.peek(&mut byte) {
                        Ok(0) => false,
                        Ok(_) => true,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                        Err(_) => false,
                    };
                    output.set_nonblocking(false)?;
                    anyhow::ensure!(alive, "player disconnected");
                    continue;
                }
            };
            anyhow::ensure!(n > 0, "unexpected end of verified video");
            let mut sent = 0;
            while sent < n {
                cx.checkpoint()?;
                anyhow::ensure!(
                    !stopping.load(Ordering::Acquire)
                        && !self.inner.stopping.load(Ordering::Acquire),
                    "playback stopped"
                );
                match output.write(&buffer[sent..n]) {
                    Ok(0) => bail!("player disconnected"),
                    Ok(count) => sent += count,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            remaining -= n as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_video_container_uses_its_own_media_type() {
        for (extension, mime) in [
            ("MP4", "video/mp4"),
            ("m4v", "video/mp4"),
            ("mov", "video/quicktime"),
            ("mkv", "video/x-matroska"),
            ("webm", "video/webm"),
            ("avi", "video/x-msvideo"),
            ("ts", "video/mp2t"),
        ] {
            assert_eq!(media_type(Path::new(&format!("movie.{extension}"))), mime);
        }
    }
}
