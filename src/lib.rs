use std::collections::BTreeMap;
use std::time::Duration;
#[cfg(feature = "worker-sdk")]
use worker::Date;
use worker_kv::KvStore;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] worker_kv::KvError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(feature = "worker-sdk")]
impl From<Error> for worker::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::Storage(error) => error.into(),
            Error::Json(error) => error.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq)]
pub enum Permit {
    Allow(Option<Ticket>),
    Deny,
}

pub type Stamp = BTreeMap<u64, u64>;

pub async fn fetch(kv: &KvStore, key: &str) -> Result<Stamp> {
    let stamp = if let Some(bytes) = kv.get(key).bytes().await? {
        serde_json::from_slice::<Stamp>(&bytes)?
    } else {
        Stamp::default()
    };
    Ok(stamp)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Datetime {
    pub timestamp: u64,
}

impl Datetime {
    pub fn from_timestamp(timestamp: u64) -> Self {
        Self { timestamp }
    }
}

#[cfg(feature = "worker-sdk")]
impl From<&Date> for Datetime {
    fn from(date: &Date) -> Self {
        Self::from_timestamp(date.as_millis() / 1000)
    }
}

pub struct RateLimiter {
    pub prefix: String,
    pub rules: BTreeMap<Duration, u64>,
}

impl RateLimiter {
    pub fn new<I: Into<String>>(prefix: I) -> Self {
        Self {
            prefix: prefix.into(),
            rules: BTreeMap::new(),
        }
    }

    pub fn add_limit(&mut self, duration: Duration, amount: u64) {
        self.rules.insert(duration, amount);
    }

    pub fn check_stamp<D: Into<Datetime>>(
        &self,
        stamp: &Stamp,
        now: D,
    ) -> (Permit, Option<Duration>) {
        let now = now.into();

        let mut max = None;
        for (duration, amount) in &self.rules {
            let start = now.timestamp - duration.as_secs();
            let end = now.timestamp;

            let mut sum = 0;
            for (_timestamp, num) in stamp.range(start..=end) {
                sum += num;
            }

            if sum >= *amount {
                return (Permit::Deny, None);
            }

            max = Some(*duration);
        }
        (Permit::Allow(None), max)
    }

    pub async fn check_kv<D: Into<Datetime>>(
        &self,
        kv: &KvStore,
        ip_addr: &str,
        now: D,
    ) -> Result<Permit> {
        let now = now.into();

        let key = format!("{}/{}", self.prefix, ip_addr);
        let stamp = fetch(kv, &key).await?;
        let (mut permit, max) = self.check_stamp(&stamp, now);

        // if the action is allowed, and there was at least one rule set, issue a ticket
        if let (Permit::Allow(ticket), Some(max)) = (&mut permit, max) {
            *ticket = Some(Ticket {
                key,
                datetime: now,
                max,
            });
        }

        Ok(permit)
    }
}

#[derive(Debug, PartialEq)]
pub struct Ticket {
    pub key: String,
    pub datetime: Datetime,
    pub max: Duration,
}

impl Ticket {
    fn expire(&self, stamp: &mut Stamp) {
        let cutoff = self.datetime.timestamp - self.max.as_secs();
        *stamp = stamp.split_off(&cutoff);
    }

    pub async fn redeem(self, kv: &KvStore) -> Result<()> {
        let mut stamp = fetch(kv, &self.key).await?;
        self.expire(&mut stamp);

        let counter = stamp.entry(self.datetime.timestamp).or_default();
        *counter = counter.saturating_add(1);

        let bytes = serde_json::to_vec(&stamp)?;
        kv.put_bytes(&self.key, &bytes)?
            .expiration_ttl(self.max.as_secs() + 1)
            .execute()
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_compatible_types() {
        let _: Option<worker_kv::KvStore> = Option::<worker::kv::KvStore>::None;
    }

    #[test]
    fn test_stamp_check_allow_empty() {
        let mut limits = RateLimiter::new("ratelimit");
        limits.add_limit(Duration::from_secs(5), 2);

        let stamp: Stamp = [].into_iter().collect();
        let date = Datetime::from_timestamp(1710528366);
        let (permit, _) = limits.check_stamp(&stamp, date);
        assert_eq!(permit, Permit::Allow(None));
    }

    #[test]
    fn test_stamp_check_allow_some() {
        let mut limits = RateLimiter::new("ratelimit");
        limits.add_limit(Duration::from_secs(5), 2);

        let stamp: Stamp = [(1710528362, 1)].into_iter().collect();
        let date = Datetime::from_timestamp(1710528366);
        let (permit, _) = limits.check_stamp(&stamp, date);
        assert_eq!(permit, Permit::Allow(None));
    }

    #[test]
    fn test_stamp_check_deny() {
        let mut limits = RateLimiter::new("ratelimit");
        limits.add_limit(Duration::from_secs(5), 2);

        let stamp: Stamp = [(1710528364, 1), (1710528363, 1)].into_iter().collect();
        let date = Datetime::from_timestamp(1710528366);
        let (permit, _) = limits.check_stamp(&stamp, date);
        assert_eq!(permit, Permit::Deny);
    }

    #[test]
    fn test_expire_stamp() {
        let mut stamp: Stamp = [
            (1710550615, 3),
            (1710550614, 4),
            (1710550613, 7),
            (1710550612, 1),
            (1710550611, 9),
        ]
        .into_iter()
        .collect();
        let ticket = Ticket {
            key: "abc".to_string(),
            datetime: Datetime::from_timestamp(1710550643),
            max: Duration::from_secs(30),
        };
        ticket.expire(&mut stamp);
        let expected: Stamp = [(1710550615, 3), (1710550614, 4), (1710550613, 7)]
            .into_iter()
            .collect();
        assert_eq!(stamp, expected);
    }
}
