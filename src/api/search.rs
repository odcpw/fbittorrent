//! Search wire objects and strict operation arguments. Unknown measurements are absent.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchItem {
    pub key: String,
    pub name: String,
    pub providers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leechers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magnet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<Hash>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchStartArgs {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchIdArgs {
    pub search_id: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProviderArgs {
    pub name: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInstallArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchBrowseArgs {
    pub source: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchGrabArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<SearchItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchStarted {
    pub search_id: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Nova,
    Torznab,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub name: String,
    pub kind: ProviderKind,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidersResult {
    pub providers: Vec<Provider>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallFailure {
    pub source: String,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledResult {
    pub installed: Vec<String>,
    pub failed: Vec<InstallFailure>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InstallReply {
    Pending(PendingResult),
    Complete(InstalledResult),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowseSource {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub in_app: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowseSourcesResult {
    pub sources: Vec<BrowseSource>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchItemsResult {
    pub items: Vec<SearchItem>,
}

impl SearchItem {
    pub fn validate(&self) -> validation::Checked {
        validation::text(&self.key, 2048, "key")?;
        validation::text(&self.name, 1024, "name")?;
        if self.key.is_empty()
            || self.name.is_empty()
            || self.providers.is_empty()
            || self.providers.len() > 256
            || self
                .rating
                .is_some_and(|v| !v.is_finite() || !(0.0..=10.0).contains(&v))
        {
            return Err(validation::Violation(
                "result: invalid identity, providers or rating".into(),
            ));
        }
        for p in &self.providers {
            validation::text(p, 256, "providers")?;
        }
        for s in [&self.magnet, &self.url, &self.page_url, &self.provider_url]
            .into_iter()
            .flatten()
        {
            validation::text(s, 8192, "url")?;
        }
        Ok(())
    }
}

pub fn validate_call(call: &Call) -> validation::Checked {
    let invalid = |key: &str| validation::Violation(format!("{key}: invalid search argument"));
    match call {
        Call::SearchStart { args } => {
            validation::text(&args.query, 256, "query")?;
            if args.query.trim().is_empty() {
                return Err(invalid("query"));
            }
            if args.limit.is_some_and(|n| n == 0 || n > 5000) {
                return Err(invalid("limit"));
            }
            if let Some(providers) = &args.providers {
                validation::count(providers.len(), 256, "providers")?;
                for p in providers {
                    validation::text(p, 256, "providers")?;
                }
            }
        }
        Call::SearchStop { args } if args.search_id == 0 => return Err(invalid("search_id")),
        Call::SearchSetProvider { args } => {
            validation::text(&args.name, 256, "name")?;
            if args.name.is_empty()
                || args.name.starts_with('_')
                || !args
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(invalid("name"));
            }
        }
        Call::SearchInstall { args } => {
            if (args.curated == Some(true)) == args.sources.is_some() {
                return Err(invalid("sources"));
            }
            if let Some(sources) = &args.sources {
                validation::count(sources.len(), 32, "sources")?;
                for s in sources {
                    validation::text(s, 8192, "sources")?;
                    if !url::Url::parse(s)
                        .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
                    {
                        return Err(invalid("sources"));
                    }
                }
            }
        }
        Call::SearchGrab { args } => {
            let selected = args.search_id.is_some_and(|n| n > 0)
                && args.key.as_ref().is_some_and(|s| !s.is_empty());
            if selected == args.result.is_some()
                || (args.result.is_some() && (args.search_id.is_some() || args.key.is_some()))
            {
                return Err(invalid("result"));
            }
            if let Some(key) = &args.key {
                validation::text(key, 2048, "key")?;
            }
            if let Some(result) = &args.result {
                result.validate()?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_search_arguments_and_limits() {
        for args in [
            json!({"query":""}),
            json!({"query":"x","limit":0}),
            json!({"query":"x","limit":5001}),
            json!({"query":"x","providers":[]}),
            json!({"query":"x","typo":true}),
            json!({"query":"x".repeat(257)}),
        ] {
            assert!(
                serde_json::from_value::<Request>(json!({"id":1,"op":"search.start","args":args}))
                    .is_err(),
                "{args}"
            );
        }
        for args in [
            json!({}),
            json!({"curated":true,"sources":["https://example.invalid/a.py"]}),
            json!({"sources":["http://example.invalid/a.py"]}),
        ] {
            assert!(serde_json::from_value::<Request>(
                json!({"id":1,"op":"search.install","args":args})
            )
            .is_err());
        }
        assert!(serde_json::from_value::<Request>(
            json!({"id":1,"op":"search.start","args":{"query":"x","limit":5000}})
        )
        .is_ok());
        assert!(serde_json::from_value::<Topic>(json!("search:0")).is_err());
        assert_eq!(
            serde_json::from_value::<Topic>(json!("search:42")).unwrap(),
            Topic::Search(42)
        );
    }
    #[test]
    fn search_topic_checks_keyed_items_and_optional_measurements() {
        let value = json!({"ev":"snapshot.chunk","sub":1,"topic":"search:1","rev":1,"items":[{"key":"a","name":"Sintel","providers":["alpha"]}]});
        let event: Event = serde_json::from_value(value.clone()).unwrap();
        event.validate().unwrap();
        assert_eq!(serde_json::to_value(event).unwrap(), value);
        let mut wrong = value.clone();
        wrong["topic"] = json!("torrents");
        assert!(serde_json::from_value::<Event>(wrong)
            .unwrap()
            .validate()
            .is_err());
    }
}
