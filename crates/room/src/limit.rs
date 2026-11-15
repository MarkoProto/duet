//! Token buckets: each person may burst a little, then is held to a steady rate.

#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    tokens: f64,
    cap: f64,
    per_ms: f64,
    last_ms: u64,
}

impl Bucket {
    pub const fn new(cap: f64, per_second: f64) -> Self {
        Self {
            tokens: cap,
            cap,
            per_ms: per_second / 1000.0,
            last_ms: 0,
        }
    }

    /// Takes one token if there is one.
    pub fn take(&mut self, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_ms) as f64;
        self.last_ms = now_ms;
        self.tokens = (self.tokens + elapsed * self.per_ms).min(self.cap);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One person's limits, by kind of message.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub chat: Bucket,
    pub react: Bucket,
    pub act: Bucket,
    pub seal: Bucket,
    pub live: Bucket,
    pub nudge: Bucket,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            chat: Bucket::new(6.0, 1.5),
            react: Bucket::new(20.0, 6.0),
            act: Bucket::new(90.0, 45.0),
            seal: Bucket::new(10.0, 3.0),
            live: Bucket::new(240.0, 120.0),
            nudge: Bucket::new(2.0, 0.25),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_then_holds_the_rate() {
        let mut b = Bucket::new(3.0, 1.0);
        assert!(b.take(0) && b.take(0) && b.take(0));
        assert!(!b.take(0));
        assert!(!b.take(500));
        assert!(b.take(1500));
        assert!(!b.take(1500));
    }

    #[test]
    fn never_holds_more_than_its_cap() {
        let mut b = Bucket::new(2.0, 10.0);
        assert!(b.take(1_000_000));
        assert!(b.take(1_000_000));
        assert!(!b.take(1_000_000));
    }
}
