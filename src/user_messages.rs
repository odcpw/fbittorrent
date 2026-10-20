//! Public failure explanations shared by CLI, API and pushed torrent rows.
pub const INVALID_MAGNET: &str = "Invalid magnet link. Copy the full magnet link from its source and try again.";
pub const INVALID_TORRENT: &str = "This .torrent file is damaged or invalid. Download it again from its source.";
pub const UNREADABLE_TORRENT: &str = "Cannot read the .torrent file. Check that it exists and is readable, or choose another file.";
pub const DISK_FULL: &str = "Disk full. Free space in the download folder, then retry.";
pub const PERMISSION: &str = "Cannot write to the download folder. Check its permissions or choose another folder, then retry.";
pub const MISSING: &str = "The download folder or a file is missing. Restore it or choose another folder, then retry.";
pub const TOO_LONG: &str = "The file path is too long for this filesystem. Choose a shorter download folder or another torrent.";
pub const METADATA: &str = "No peers have supplied the file list yet. Check Trackers, retry later or try another source.";
pub const TRACKERS: &str = "All trackers failed. Check your connection or the Trackers tab, then retry or add another tracker.";
pub const PROTOCOL: &str = "The daemon and plugin use different API versions. Run fbittorrent setup --plugin=copy, then restart the shell.";

pub fn storage(text: &str) -> Option<&'static str> {
    let text=text.to_ascii_lowercase();
    if text.contains("no space left on device") || text.contains("os error 28") || text.contains("enospc") {Some(DISK_FULL)}
    else if text.contains("permission denied") || text.contains("read-only file system") || text.contains("os error 13") {Some(PERMISSION)}
    else if text.contains("file name too long") || text.contains("filename too long") || text.contains("os error 36") {Some(TOO_LONG)}
    else if text.contains("no such file or directory") || text.contains("missing files") || text.contains("os error 2)") {Some(MISSING)}
    else {None}
}

pub fn row(row: &crate::api::Torrent) -> Option<&str> {
    if let Some(error)=row.error.as_deref() {return Some(error);}
    if row.reason.as_deref()==Some("metadata_timeout") || row.state==crate::api::TorrentState::Metadata || (row.staged && row.size.is_none()) {return Some(METADATA);}
    if row.state==crate::api::TorrentState::Stalled {return Some("No peers are sending data. Check Trackers, retry later or try another source.");}
    None
}

pub fn torrent_error(reason:Option<&str>,error:Option<&str>)->Option<String> {
    let message=match reason {
        Some("disk_full")=>Some(DISK_FULL),
        Some("missing_files")=>Some(MISSING),
        Some("metadata_timeout")=>Some(METADATA),
        _=>error.map(|text|if text==TRACKERS {TRACKERS}else{storage(text).unwrap_or("The torrent could not continue. Open Log for details, then retry.")}),
    };
    message.map(str::to_owned)
}
