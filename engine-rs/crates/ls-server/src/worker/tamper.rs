use std::time::{Duration, Instant};

const WIDTH: usize = 80;
const HEIGHT: usize = 45;
const INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum ImageHealth {
    #[default]
    Unknown,
    Normal,
    Dark,
    Bright,
    Frozen,
}

impl ImageHealth {
    pub fn label(index: u8) -> &'static str {
        match index {
            x if x == Self::Normal as u8 => "normal",
            x if x == Self::Dark as u8 => "dark",
            x if x == Self::Bright as u8 => "bright",
            x if x == Self::Frozen as u8 => "frozen",
            _ => "unknown",
        }
    }

    pub fn description(self) -> Option<&'static str> {
        match self {
            Self::Dark => Some("画面持续近黑，可能被遮挡"),
            Self::Bright => Some("画面持续过亮，可能受到强光照射"),
            Self::Frozen => Some("画面曾变化，随后长时间完全相同，可能冻结"),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct ImageMonitor {
    current: ImageHealth,
    pending: Option<(ImageHealth, Instant)>,
    last_sample: Option<Instant>,
    last_change: Option<Instant>,
    previous: Vec<u8>,
}

impl ImageMonitor {
    pub fn observe(&mut self, frame: &ls_ingest::Frame, now: Instant) -> Option<ImageHealth> {
        if self
            .last_sample
            .is_some_and(|t| now.saturating_duration_since(t) < INTERVAL)
        {
            return None;
        }
        let (w, h) = (frame.width as usize, frame.height as usize);
        if w == 0 || h == 0 || frame.data.len() != w.checked_mul(h)?.checked_mul(3)? {
            return None;
        }
        self.last_sample = Some(now);
        let mut sampled = Vec::with_capacity(WIDTH * HEIGHT);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let i = ((2 * y + 1) * h / (2 * HEIGHT) * w + (2 * x + 1) * w / (2 * WIDTH)) * 3;
                let [b, g, r] =
                    [frame.data[i], frame.data[i + 1], frame.data[i + 2]].map(u16::from);
                sampled.push(((29 * b + 150 * g + 77 * r) >> 8) as u8);
            }
        }
        self.sample(sampled, now)
    }

    fn sample(&mut self, gray: Vec<u8>, now: Instant) -> Option<ImageHealth> {
        let high = gray.iter().filter(|value| **value >= 250).count();
        let low = gray.iter().filter(|value| **value <= 6).count();
        if !self.previous.is_empty() && self.previous != gray {
            self.last_change = Some(now);
        }
        let next = if low * 100 >= gray.len() * 99 {
            ImageHealth::Dark
        } else if high * 100 >= gray.len() * 99 {
            ImageHealth::Bright
        } else if self
            .last_change
            .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(15))
        {
            ImageHealth::Frozen
        } else {
            ImageHealth::Normal
        };
        self.previous = gray;
        if next == self.current {
            self.pending = None;
            return None;
        }
        if self.pending.is_none_or(|(state, _)| state != next) {
            self.pending = Some((next, now));
        }
        let (_, since) = self.pending?;
        if self.current != ImageHealth::Unknown || next != ImageHealth::Normal {
            let hold = if next == ImageHealth::Normal { 2 } else { 4 };
            if now.saturating_duration_since(since) < Duration::from_secs(hold) {
                return None;
            }
        }
        self.current = next;
        self.pending = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_room_and_temporary_night_switch_do_not_raise_an_alarm() {
        let start = Instant::now();
        let mut monitor = ImageMonitor::default();
        let mut sample = |t, pixel| monitor.sample(vec![pixel; 16], start + Duration::from_secs(t));
        assert_eq!(sample(0, 42), Some(ImageHealth::Normal));
        assert_eq!(sample(60, 42), None);
        assert_eq!(sample(61, 0), None);
        assert_eq!(sample(62, 22), None);
        assert_eq!(sample(63, 23), None);
        assert_eq!(sample(77, 23), None);
        assert_eq!(sample(78, 23), None);
        assert_eq!(sample(82, 23), Some(ImageHealth::Frozen));
        assert_eq!(sample(83, 31), None);
        assert_eq!(sample(85, 31), Some(ImageHealth::Normal));
    }
}
