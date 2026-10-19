//! RSS wire objects and strict arguments (P3-02).
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    pub id:String, pub name:String, pub url:String,
    #[serde(skip_serializing_if="Option::is_none")] pub title:Option<String>,
    pub unread:u64,
    #[serde(skip_serializing_if="Option::is_none")] pub last_refresh_at:Option<u64>,
    pub loading:bool,
    #[serde(skip_serializing_if="Option::is_none")] pub error:Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedItem {
    pub id:String, pub title:String,
    #[serde(skip_serializing_if="Option::is_none")] pub description:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub published_at:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub size:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")] pub link:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub magnet:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub torrent_url:Option<String>,
    pub read:bool, pub grabbed:bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedArgs { pub feed:String }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedAddArgs { pub url:String, #[serde(skip_serializing_if="Option::is_none")] pub name:Option<String> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedRefreshArgs { #[serde(skip_serializing_if="Option::is_none")] pub feed:Option<String> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedItemsArgs { pub feed:String, #[serde(skip_serializing_if="Option::is_none")] pub unread_only:Option<bool> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedReadArgs { pub feed:String, pub item:String, pub read:bool }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedGrabArgs {
    pub feed:String, pub item:String,
    #[serde(skip_serializing_if="Option::is_none")] pub seed:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub stopped:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")] pub save_path:Option<String>,
    #[serde(skip_serializing_if="Option::is_none")] pub files:Option<Vec<u64>>,
    #[serde(skip_serializing_if="Option::is_none")] pub category:Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct FeedsResult { pub feeds:Vec<Feed> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct FeedResult { pub feed:Feed }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct FeedItemsResult { pub items:Vec<FeedItem> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct CountResult { pub count:u64 }

impl validation::Validate for Feed {
    fn validate(&self)->validation::Checked { validation::text(&self.id,LINE_IN,"id") }
}
impl validation::Validate for FeedItem {
    fn validate(&self)->validation::Checked { validation::text(&self.id,LINE_IN,"id") }
}
