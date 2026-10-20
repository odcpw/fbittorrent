//! Keep packet bursts and short zero-rate gaps out of the playback verdict.
use super::{Playable, Sample};
use std::time::Instant;

#[derive(Clone, Default)]
pub(super) struct Estimate {
    rate: Option<f64>,
    sampled: Option<Instant>,
    verified: Option<u64>,
    limit: Option<Option<u64>>,
    settling_until: Option<Instant>,
    safe: Option<bool>,
}
impl Estimate {
    pub fn apply(
        &mut self,
        sample: &Sample,
        seconds: Option<f64>,
        now: Instant,
        value: &mut Playable,
    ) {
        let elapsed = self
            .sampled
            .map_or(1., |last| now.saturating_duration_since(last).as_secs_f64());
        self.sampled = Some(now);
        let limit_changed = self
            .limit
            .replace(sample.download_limit)
            .is_some_and(|previous| previous != sample.download_limit);
        if limit_changed {
            self.settling_until = Some(now + std::time::Duration::from_secs(3));
        }
        let verified_rate = self
            .verified
            .filter(|old| sample.verified_bytes > *old && elapsed > 0.)
            .map_or(0., |old| (sample.verified_bytes - old) as f64 / elapsed);
        self.verified = Some(sample.verified_bytes);
        let fresh_rate =
            self.settling_until.is_some_and(|until| now <= until) && verified_rate > 0.;
        if fresh_rate {
            self.settling_until = None;
        }
        if let Some(rate) = sample.dl_bps {
            // Speed counters lag a changed cap. Newly verified payload
            // bytes are direct evidence that the link is now moving faster.
            let rate = (rate as f64).max(verified_rate);
            self.rate = Some(if fresh_rate {
                rate
            } else {
                self.rate.map_or(rate, |old| {
                    old + (rate - old) * (1. - (-elapsed / 5.).exp())
                })
            });
        }
        // Average token-bucket bursts before applying the sustained cap. Clipping
        // each burst first would undercount a healthy burst / zero / burst link.
        let rate = self.rate.map(|rate| {
            sample
                .download_limit
                .map_or(rate, |cap| rate.min(cap as f64))
        });
        let remaining = sample.size.saturating_sub(sample.prefix);
        value.finishes_in_s = rate
            .filter(|r| *r >= 1. && sample.active)
            .map(|rate| (remaining as f64 / rate).ceil() as u64);
        let Some(duration) = seconds.filter(|_| sample.active) else {
            value.stall_free = None;
            return;
        };
        let Some(rate) = rate else {
            value.stall_free = None;
            return;
        };
        let buffer = sample.prefix as f64 * duration / sample.size.max(1) as f64;
        let finish = if remaining == 0 {
            0.
        } else if rate > 0. {
            remaining as f64 / rate
        } else {
            f64::INFINITY
        };
        let verdict = match self.safe {
            None => buffer >= finish,
            Some(true) => buffer >= finish,
            Some(false) => buffer >= finish * 1.1,
        };
        // The two thresholds are the hysteresis. A second timer here would
        // delay a confirmed speed-up by another entire sampling interval.
        self.safe = Some(verdict);
        value.stall_free = self.safe;
    }
}
