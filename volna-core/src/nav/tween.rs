//! An eased animation over any interpolable value, driven by an explicit clock so tests are
//! deterministic and frontends only request frames while something moves.

use web_time::Instant;

pub trait Lerp: Clone {
    fn lerp(&self, to: &Self, t: f64) -> Self;
    fn approx_eq(&self, other: &Self) -> bool;
}

#[derive(Clone)]
pub struct Tween<T: Lerp> {
    pub value: T,
    animation: Option<Animation<T>>,
}

#[derive(Clone)]
struct Animation<T> {
    from: T,
    to: T,
    start: Instant,
    duration: std::time::Duration,
}

impl<T: Lerp> Tween<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            animation: None,
        }
    }

    pub fn is_animating(&self) -> bool {
        self.animation.is_some()
    }

    pub fn target(&self) -> T {
        self.animation
            .as_ref()
            .map_or_else(|| self.value.clone(), |a| a.to.clone())
    }

    pub(crate) fn map(&mut self, f: impl Fn(&T) -> T) {
        self.value = f(&self.value);
        if let Some(a) = &mut self.animation {
            a.from = f(&a.from);
            a.to = f(&a.to);
        }
    }

    pub fn set(&mut self, value: T) {
        self.value = value;
        self.animation = None;
    }

    pub fn animate_to(&mut self, target: T, now: Instant, mode: crate::settings::Animation) {
        self.tick(now);
        let Some(duration) = mode.duration() else {
            self.set(target);
            return;
        };
        self.animation = if self.value.approx_eq(&target) {
            None
        } else {
            Some(Animation {
                from: self.value.clone(),
                to: target,
                start: now,
                duration,
            })
        };
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(a) = &self.animation else {
            return false;
        };
        let t = (now.saturating_duration_since(a.start).as_secs_f64() / a.duration.as_secs_f64())
            .min(1.0);
        self.value = a.from.lerp(&a.to, 1.0 - (1.0 - t).powi(3));
        if t >= 1.0 {
            self.value = a.to.clone();
            self.animation = None;
        }
        self.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Animation;
    use std::time::Duration;

    impl Lerp for f64 {
        fn lerp(&self, to: &Self, t: f64) -> Self {
            self + (to - self) * t
        }
        fn approx_eq(&self, other: &Self) -> bool {
            (self - other).abs() < 1e-9
        }
    }

    #[test]
    fn eases_to_the_target_and_reports_when_done() {
        let now = Instant::now();
        let mut t = Tween::new(0.0);
        t.animate_to(10.0, now, Animation::On);
        assert!(t.is_animating());
        assert_eq!(t.target(), 10.0);
        assert!(t.tick(now + Duration::from_millis(70)));
        assert!(t.value > 0.0 && t.value < 10.0);
        assert!(!t.tick(now + Duration::from_secs(1)));
        assert_eq!(t.value, 10.0);
        t.animate_to(10.0, now, Animation::On);
        assert!(!t.is_animating(), "no animation to the same value");
        t.animate_to(20.0, now, Animation::Off);
        assert_eq!(t.value, 20.0);
        assert!(!t.is_animating());
        t.animate_to(30.0, now, Animation::Reduced);
        t.set(1.0);
        assert!(!t.is_animating());
        assert_eq!(t.target(), 1.0);
    }
}
