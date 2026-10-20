//! Public failure explanations shared by CLI, API and pushed torrent rows.
pub const INVALID_MAGNET: &str = "Invalid magnet link. Copy the full magnet link from its source and try again.";
pub const INVALID_TORRENT: &str = "This .torrent file is damaged or invalid. Download it again from its source.";
pub const UNREADABLE_TORRENT: &str = "Cannot read the .torrent file. Check that it exists and is readable, or choose another file.";
