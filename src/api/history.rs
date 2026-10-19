//! Ordered real rate samples; no padding or interpolation.
use serde::{Serialize,Deserialize};
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateHistory {pub download:Vec<u64>,pub upload:Vec<u64>,pub interval_s:u64}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampledStats {pub history:RateHistory,
    #[serde(skip_serializing_if="Option::is_none")] pub session:Option<super::SessionTotals>,
    #[serde(skip_serializing_if="Option::is_none")] pub all_time:Option<super::AllTimeTotals>,
}
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsArgs {#[serde(skip_serializing_if="Option::is_none")]pub history:Option<bool>}
impl super::validation::Validate for RateHistory {
    fn validate(&self)->super::validation::Checked {
        if self.download.len()>600||self.download.len()!=self.upload.len() {return Err(super::validation::Violation("history: matching download/upload arrays, at most 600 samples".into()));}
        if self.interval_s!=1 {return Err(super::validation::Violation("history.interval_s: one second".into()));}
        Ok(())
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use crate::api::{self,validation::{Validate,NegotiatedCaps}};
    #[test] fn bounds_and_capability() {
        let mut history=RateHistory {download:vec![0],upload:vec![0],interval_s:1};
        history.validate().unwrap();history.upload.clear();assert!(history.validate().is_err());
        history.download=vec![0;601];history.upload=vec![0;601];assert!(history.validate().is_err());
        history.download.clear();history.upload.clear();history.interval_s=2;assert!(history.validate().is_err());
        history.interval_s=1;
        let frame=api::Event::State {sub:1,topic:api::Topic::Stats,rev:1,data:api::StateData::Stats(SampledStats {history,session:None,all_time:None})};
        frame.validate().unwrap();assert!(NegotiatedCaps::default().event(&frame).is_err());
        let hello:serde_json::Value=include_str!("../../contracts/ipc/v1/hello.ndjson").lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|r|r["frame"]["result"].get("caps").is_some()).unwrap();
        let hello:api::HelloResult=serde_json::from_value(hello["frame"]["result"].clone()).unwrap();
        NegotiatedCaps::from_hello(&hello).event(&frame).unwrap();
    }
}
