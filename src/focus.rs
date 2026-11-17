use serde::{Deserialize, Serialize};
use std::time::Duration;

pub type Features = [f64; 4];
const FLOORS: Features = [0.045, 0.055, 0.04, 0.055];
pub const NAMES: [&str; 4] = ["Monitor 1", "Monitor 2", "Notebook", "Phone"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Zone {
    pub mean: Features,
    pub spread: Features,
}
impl Zone {
    pub fn valid(&self) -> bool {
        self.mean.iter().all(|x| x.is_finite() && x.abs() < 10.)
            && self
                .spread
                .iter()
                .zip(FLOORS)
                .all(|(x, floor)| x.is_finite() && *x >= floor && *x <= 0.24)
    }
    pub fn fit(samples: &[Features]) -> Option<Self> {
        if samples.len() < 20
            || samples
                .iter()
                .flatten()
                .any(|x| !x.is_finite() || x.abs() >= 10.)
        {
            return None;
        }
        let mut zone = Self {
            mean: [0.; 4],
            spread: [0.; 4],
        };
        for sample in samples {
            for (i, value) in sample.iter().enumerate() {
                zone.mean[i] += value / samples.len() as f64;
            }
        }
        for sample in samples {
            for (i, value) in sample.iter().enumerate() {
                zone.spread[i] += (value - zone.mean[i]).powi(2) / samples.len() as f64;
            }
        }
        for (spread, floor) in zone.spread.iter_mut().zip(FLOORS) {
            *spread = spread.sqrt().max(floor);
        }
        zone.valid().then_some(zone)
    }
    pub fn distance(&self, sample: &Features) -> f64 {
        if !self.valid() || sample.iter().any(|x| !x.is_finite()) {
            return f64::INFINITY;
        }
        (sample
            .iter()
            .enumerate()
            .map(|(i, value)| ((value - self.mean[i]) / self.spread[i]).powi(2))
            .sum::<f64>()
            / 4.)
            .sqrt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attention {
    Working(usize),
    Distracted,
    Phone,
    Unknown,
}

pub fn classify(
    sample: Option<&Features>,
    zones: &[Option<Zone>; 4],
    notes: bool,
    tolerance: f64,
) -> Attention {
    let Some(sample) = sample else {
        return Attention::Unknown;
    };
    if zones[0].is_none() || zones[1].is_none() || sample.iter().any(|x| !x.is_finite()) {
        return Attention::Unknown;
    }
    let mut best = (usize::MAX, f64::INFINITY);
    for (i, zone) in zones.iter().enumerate().take(3) {
        if i == 2 && !notes {
            continue;
        }
        if let Some(zone) = zone {
            let distance = zone.distance(sample);
            if distance < best.1 {
                best = (i, distance);
            }
        }
    }
    let phone = zones[3]
        .as_ref()
        .map_or(f64::INFINITY, |zone| zone.distance(sample));
    if phone <= tolerance && best.1 <= tolerance && (phone - best.1).abs() < 0.5 {
        return Attention::Unknown;
    }
    if phone <= tolerance && phone < best.1 {
        return Attention::Phone;
    }
    if best.1 <= tolerance {
        Attention::Working(best.0)
    } else {
        Attention::Distracted
    }
}

#[derive(Default)]
pub struct AlertGate {
    since: Option<Duration>,
    last_alert: Option<Duration>,
    last_sample: Option<Duration>,
}
impl AlertGate {
    pub fn update(
        &mut self,
        attention: Attention,
        now: Duration,
        delay: Duration,
        cooldown: Duration,
    ) -> bool {
        if self.last_sample.is_some_and(|last| {
            now < last || now.saturating_sub(last) > Duration::from_millis(1500)
        }) {
            self.since = None;
        }
        self.last_sample = Some(now);
        if !matches!(attention, Attention::Distracted | Attention::Phone) {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if now.saturating_sub(since) >= delay
            && self
                .last_alert
                .is_none_or(|last| now.saturating_sub(last) >= cooldown)
        {
            self.last_alert = Some(now);
            self.since = Some(now);
            return true;
        }
        false
    }
    pub fn progress(&self, now: Duration, delay: Duration) -> f64 {
        self.since.map_or(0., |since| {
            (now.saturating_sub(since).as_secs_f64() / delay.as_secs_f64().max(0.1)).clamp(0., 1.)
        })
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seconds(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn zone(offset: f64) -> Zone {
        Zone::fit(&[[offset; 4]; 30]).unwrap()
    }
    #[test]
    fn rejects_bad_calibration() {
        assert!(Zone::fit(&[[0.; 4]; 19]).is_none());
        assert!(Zone::fit(&[[f64::NAN; 4]; 30]).is_none());
        let samples: Vec<_> = (0..30).map(|i| [i as f64; 4]).collect();
        assert!(Zone::fit(&samples).is_none());
        let mut bad = zone(0.);
        bad.spread[0] = 0.;
        assert!(!bad.valid());
    }
    #[test]
    fn separates_work_phone_and_uncertainty() {
        let zones = [
            Some(zone(0.)),
            Some(zone(0.4)),
            Some(zone(0.8)),
            Some(zone(1.2)),
        ];
        assert_eq!(
            classify(Some(&[0.; 4]), &zones, true, 2.5),
            Attention::Working(0)
        );
        assert_eq!(
            classify(Some(&[0.4; 4]), &zones, true, 2.5),
            Attention::Working(1)
        );
        assert_eq!(
            classify(Some(&[0.8; 4]), &zones, true, 2.5),
            Attention::Working(2)
        );
        assert_eq!(
            classify(Some(&[0.8; 4]), &zones, false, 2.5),
            Attention::Distracted
        );
        assert_eq!(
            classify(Some(&[1.2; 4]), &zones, true, 2.5),
            Attention::Phone
        );
        assert_eq!(classify(None, &zones, true, 2.5), Attention::Unknown);
        assert_eq!(
            classify(Some(&[f64::NAN; 4]), &zones, true, 2.5),
            Attention::Unknown
        );
        let overlap = [Some(zone(0.)), Some(zone(0.4)), None, Some(zone(0.))];
        assert_eq!(
            classify(Some(&[0.; 4]), &overlap, true, 2.5),
            Attention::Unknown
        );
    }
    #[test]
    fn brief_glances_unknown_and_missing_frames_do_not_trigger() {
        let mut gate = AlertGate::default();
        for n in 0..5 {
            assert!(!gate.update(Attention::Distracted, seconds(n), seconds(5), seconds(25)));
        }
        assert!(!gate.update(Attention::Unknown, seconds(5), seconds(5), seconds(25)));
        assert!(!gate.update(Attention::Distracted, seconds(6), seconds(5), seconds(25)));
        assert!(!gate.update(Attention::Distracted, seconds(20), seconds(5), seconds(25)));
        for n in 21..25 {
            assert!(!gate.update(Attention::Phone, seconds(n), seconds(5), seconds(25)));
        }
        assert!(gate.update(Attention::Phone, seconds(25), seconds(5), seconds(25)));
        for n in 26..50 {
            assert!(!gate.update(Attention::Phone, seconds(n), seconds(5), seconds(25)));
        }
        assert!(gate.update(Attention::Phone, seconds(50), seconds(5), seconds(25)));
        gate.reset();
        assert_eq!(gate.progress(seconds(50), seconds(5)), 0.);
    }
}
