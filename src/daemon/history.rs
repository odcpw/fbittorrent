//! One bounded global ring; queries never advance its one-second clock.
use std::{collections::VecDeque,time::{Duration,Instant}};

pub(crate) struct History {
    samples:VecDeque<(u64,u64)>,
    last:Option<Instant>,
}
impl Default for History {
    fn default()->Self {Self {samples:VecDeque::with_capacity(600),last:None}}
}
impl History {
    pub fn due(&self,now:Instant)->bool {
        self.last.is_none_or(|last|now.saturating_duration_since(last)>=Duration::from_secs(1))
    }
    pub fn record(&mut self,now:Instant,download:u64,upload:u64) {
        if !self.due(now) {return;}
        self.push(now,download,upload);
    }
    /// One point per engine tick. The tick is periodic, so a slightly early
    /// tick is not skipped; a burst after a stalled runtime collapses to one.
    pub fn record_tick(&mut self,now:Instant,download:u64,upload:u64) {
        if self.last.is_some_and(|last|now.saturating_duration_since(last)<Duration::from_millis(500)) {return;}
        self.push(now,download,upload);
    }
    fn push(&mut self,now:Instant,download:u64,upload:u64) {
        if self.samples.len()==600 {self.samples.pop_front();}
        self.samples.push_back((download,upload));self.last=Some(now);
    }
    pub fn download(&self,limit:usize)->Vec<u64> {
        self.samples.iter().skip(self.samples.len().saturating_sub(limit)).map(|s|s.0).collect()
    }
    pub fn snapshot(&self)->crate::api::RateHistory {
        crate::api::RateHistory {download:self.download(600),upload:self.samples.iter().map(|s|s.1).collect(),interval_s:1}
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ring_bound_and_order() {
        let mut ring=History::default();let start=Instant::now();
        for i in 0..700 {
            ring.record(start+Duration::from_secs(i),i,i*2);
            ring.record(start+Duration::from_secs(i)+Duration::from_millis(500),9999,9999);
        }
        let sample=ring.snapshot();println!("ring after 700 seconds: {}",serde_json::to_string(&sample).unwrap());
        assert_eq!(sample.download,(100..700).collect::<Vec<_>>());
        assert_eq!(sample.upload,(100..700).map(|i|i*2).collect::<Vec<_>>());
        assert_eq!(ring.download(60),(640..700).collect::<Vec<_>>());
        assert_eq!(sample.interval_s,1);assert_eq!(ring.samples.capacity(),600);
        assert_eq!(History::default().snapshot().download,Vec::<u64>::new());
    }
    #[test] fn engine_ticks_record_one_point_each_and_bursts_collapse() {
        let mut ring=History::default();let start=Instant::now();
        // A periodic tick may land a few milliseconds early: still one point each.
        for (i,offset) in [0u64,998,1999,3001,3999].into_iter().enumerate() {
            ring.record_tick(start+Duration::from_millis(offset),i as u64,0);
        }
        assert_eq!(ring.download(60),vec![0,1,2,3,4]);
        // Missed ticks fired back to back after a stall are one observation.
        ring.record_tick(start+Duration::from_millis(9000),5,0);
        ring.record_tick(start+Duration::from_millis(9001),6,0);
        assert_eq!(ring.download(60),vec![0,1,2,3,4,5]);
    }
}
