//! Shows API views keep read intent independent of transfer facts.
use super::*;
use serde::{Deserialize,Serialize};
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct ShowCounts {pub episodes:u64,pub seen:u64,pub grabbed:u64}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct Show {
    pub imdb:String,pub title:String,
    #[serde(skip_serializing_if="Option::is_none")] pub year:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub quality:Option<String>,
    pub unread:u64,pub added_at:u64,pub counts:ShowCounts,
    #[serde(skip_serializing_if="Option::is_none")] pub last_check_at:Option<u64>,
    pub checking:bool,pub catalogue_complete:bool,
    #[serde(skip_serializing_if="Option::is_none")] pub warning:Option<String>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct Release {
    pub key:String,pub show_imdb:String,pub title:String,
    #[serde(skip_serializing_if="Option::is_none")] pub season:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub episode:Option<u64>,
    pub season_pack:bool,
    #[serde(skip_serializing_if="Option::is_none")] pub quality:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub seeds:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub size:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub magnet:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub candidates:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub published_at:Option<u64>,
    pub read:bool,pub transfer:ReleaseTransfer,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ReleaseTransfer {None,Downloading,Queued,Paused,Done,AddedBefore}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {pub imdb:String}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowFollowArgs {
    #[serde(skip_serializing_if="Option::is_none")] pub imdb:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub query:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub quality:Option<String>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowCheckArgs {#[serde(skip_serializing_if="Option::is_none")] pub imdb:Option<String>}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowReleasesArgs {
    #[serde(skip_serializing_if="Option::is_none")] pub imdb:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub inbox:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub unread_only:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub quality:Option<String>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowReadArgs {pub imdb:String,pub key:String,pub read:bool}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowGrabArgs {
    pub imdb:String,pub key:String,
    #[serde(skip_serializing_if="Option::is_none")] pub stage:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub seed:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub stopped:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub save_path:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub files:Option<Vec<u64>>,
    #[serde(skip_serializing_if="Option::is_none")] pub category:Option<String>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)] pub struct ShowsResult {pub shows:Vec<Show>}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)] pub struct ShowResult {pub show:Show}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)] pub struct ReleasesResult {pub releases:Vec<Release>}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)] pub struct ReadTarget {pub imdb:String,pub key:String}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)] pub struct UndoReadResult {#[serde(skip_serializing_if="Option::is_none")] pub restored:Option<ReadTarget>}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(untagged)] pub enum ShowCheckResult {Pending(PendingResult),Done {new:u64}}
impl validation::Validate for Show {fn validate(&self)->validation::Checked {validation::text(&self.imdb,128,"imdb")}}
impl validation::Validate for Release {fn validate(&self)->validation::Checked {validation::text(&self.key,128,"key")}}
pub fn release_key(inbox:bool,release:&Release)->String {if inbox {format!("{}/{}",release.show_imdb,release.key)}else{release.key.clone()}}
