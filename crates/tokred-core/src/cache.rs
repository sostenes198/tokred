//! The tiered cache (table `cache_entries`): entry model, tier policy and use cases.
//!
//! | Tier | Enters when | TTL |
//! |---|---|---|
//! | L1 | first write | `l1_ttl`, fixed from creation |
//! | L2 | `hits >= l2_hits` | `l2_ttl`, renewed on every hit |
//! | L3 | `hits >= l3_hits` | persistent |
//!
//! The policy is made of pure functions over [`CacheEntry`]; the repository port only loads and
//! stores rows. Keys must include the hash of the source content, so a changed source simply
//! misses (see [`content_key`]).

use crate::config::CacheConfig;
use crate::error::StoreError;
use crate::types::Timestamp;

const DEFAULT_L1_TTL_SECS: i64 = 4 * 3600;
const DEFAULT_L2_TTL_SECS: i64 = 24 * 3600;

/// Cache tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Fresh entries.
    L1,
    /// Entries read a few times.
    L2,
    /// Entries read often; never expire.
    L3,
}

impl Tier {
    /// Every tier, lowest first.
    pub const ALL: [Self; 3] = [Self::L1, Self::L2, Self::L3];

    /// Number stored in the `tier` column.
    #[must_use]
    pub const fn as_int(self) -> i64 {
        match self {
            Self::L1 => 1,
            Self::L2 => 2,
            Self::L3 => 3,
        }
    }

    /// Parses the stored number.
    #[must_use]
    pub const fn from_int(value: i64) -> Option<Self> {
        match value {
            1 => Some(Self::L1),
            2 => Some(Self::L2),
            3 => Some(Self::L3),
            _ => None,
        }
    }
}

/// One cached value (row of `cache_entries`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// Key, including the hash of the source content.
    pub key: String,
    /// What the value is (`repo_files`, `session_warn`, ...).
    pub kind: String,
    /// The value.
    pub value: Vec<u8>,
    /// Current tier.
    pub tier: Tier,
    /// Number of reads since the first write.
    pub hits: u32,
    /// First write.
    pub created_at: Timestamp,
    /// Last read (equals `created_at` until the first hit).
    pub last_hit: Timestamp,
    /// When the entry stops being valid; `None` means persistent (L3).
    pub expires_at: Option<Timestamp>,
}

/// The tier thresholds and TTLs, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierPolicy {
    /// Time to live of an L1 entry.
    pub l1_ttl_secs: i64,
    /// Time to live of an L2 entry, renewed on every hit.
    pub l2_ttl_secs: i64,
    /// Hits needed to reach L2.
    pub l2_hits: u32,
    /// Hits needed to reach L3.
    pub l3_hits: u32,
}

impl Default for TierPolicy {
    fn default() -> Self {
        Self {
            l1_ttl_secs: DEFAULT_L1_TTL_SECS,
            l2_ttl_secs: DEFAULT_L2_TTL_SECS,
            l2_hits: 3,
            l3_hits: 10,
        }
    }
}

/// Parses a duration such as `90s`, `30m`, `4h` or `7d` into seconds.
///
/// Returns `None` for anything else, including zero, so callers can fall back to a default.
#[must_use]
pub fn parse_duration(text: &str) -> Option<i64> {
    let text = text.trim();
    let unit = text.chars().last()?;
    let factor: i64 = match unit.to_ascii_lowercase() {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return None,
    };
    let digits = &text[..text.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let amount: i64 = digits.parse().ok()?;
    (amount > 0).then(|| amount.saturating_mul(factor))
}

impl TierPolicy {
    /// Builds the policy from `[cache]`; an unreadable TTL falls back to its default, so a typo
    /// in `tokred.toml` can never break a hook.
    #[must_use]
    pub fn from_config(config: &CacheConfig) -> Self {
        Self {
            l1_ttl_secs: parse_duration(&config.l1_ttl).unwrap_or(DEFAULT_L1_TTL_SECS),
            l2_ttl_secs: parse_duration(&config.l2_ttl).unwrap_or(DEFAULT_L2_TTL_SECS),
            l2_hits: config.l2_hits,
            l3_hits: config.l3_hits,
        }
    }
}

/// Tier an entry belongs to given its `hits`.
#[must_use]
pub fn next_tier(entry: &CacheEntry, policy: &TierPolicy) -> Tier {
    tier_for_hits(entry.hits, policy)
}

fn tier_for_hits(hits: u32, policy: &TierPolicy) -> Tier {
    if hits >= policy.l3_hits {
        Tier::L3
    } else if hits >= policy.l2_hits {
        Tier::L2
    } else {
        Tier::L1
    }
}

/// Expiry of an entry in `tier`: L1 counts from `created_at`, L2 from `now` (renewed on every
/// hit), L3 never expires.
#[must_use]
pub fn expires_at(
    tier: Tier,
    created_at: Timestamp,
    now: Timestamp,
    policy: &TierPolicy,
) -> Option<Timestamp> {
    match tier {
        Tier::L1 => Some(Timestamp(created_at.0.saturating_add(policy.l1_ttl_secs))),
        Tier::L2 => Some(Timestamp(now.0.saturating_add(policy.l2_ttl_secs))),
        Tier::L3 => None,
    }
}

/// Whether the entry is no longer valid at `now`.
#[must_use]
pub fn is_expired(entry: &CacheEntry, now: Timestamp) -> bool {
    entry.expires_at.is_some_and(|at| at <= now)
}

/// A fresh entry (`hits = 0`), written at `now`.
#[must_use]
pub fn new_entry(
    key: &str,
    kind: &str,
    value: Vec<u8>,
    now: Timestamp,
    policy: &TierPolicy,
) -> CacheEntry {
    let tier = tier_for_hits(0, policy);
    CacheEntry {
        key: key.to_owned(),
        kind: kind.to_owned(),
        value,
        tier,
        hits: 0,
        created_at: now,
        last_hit: now,
        expires_at: expires_at(tier, now, now, policy),
    }
}

/// The entry after a read at `now`: one more hit, tier re-evaluated, expiry renewed per tier.
#[must_use]
pub fn record_hit(entry: &CacheEntry, now: Timestamp, policy: &TierPolicy) -> CacheEntry {
    let mut next = entry.clone();
    next.hits = entry.hits.saturating_add(1);
    next.tier = next_tier(&next, policy);
    next.last_hit = now;
    next.expires_at = expires_at(next.tier, next.created_at, now, policy);
    next
}

/// Key made of a kind, the hash of the source content and a subject.
#[must_use]
pub fn content_key(kind: &str, subject: &str, content_hash: &str) -> String {
    format!("{kind}:{content_hash}:{subject}")
}

/// Persistent cache rows.
pub trait CacheRepo {
    /// The row with this key, whatever its expiry.
    fn get_entry(&self, key: &str) -> Result<Option<CacheEntry>, StoreError>;
    /// Inserts or replaces the row.
    fn put_entry(&self, entry: &CacheEntry) -> Result<(), StoreError>;
    /// Deletes every row with `expires_at <= now`; returns how many.
    fn delete_expired(&self, now: Timestamp) -> Result<usize, StoreError>;
    /// Every row, ordered by key.
    fn list_entries(&self) -> Result<Vec<CacheEntry>, StoreError>;
}

/// Reads a live entry and counts the hit (promotion on read). Expired or missing keys miss.
pub fn cache_get(
    repo: &dyn CacheRepo,
    key: &str,
    now: Timestamp,
    policy: &TierPolicy,
) -> Result<Option<CacheEntry>, StoreError> {
    let Some(entry) = repo.get_entry(key)? else {
        return Ok(None);
    };
    if is_expired(&entry, now) {
        return Ok(None);
    }
    let promoted = record_hit(&entry, now, policy);
    repo.put_entry(&promoted)?;
    Ok(Some(promoted))
}

/// Reads a live entry without counting a hit.
pub fn cache_peek(
    repo: &dyn CacheRepo,
    key: &str,
    now: Timestamp,
) -> Result<Option<CacheEntry>, StoreError> {
    Ok(repo.get_entry(key)?.filter(|entry| !is_expired(entry, now)))
}

/// Writes a fresh entry (L1, `hits = 0`), replacing any previous one.
pub fn cache_put(
    repo: &dyn CacheRepo,
    key: &str,
    kind: &str,
    value: Vec<u8>,
    now: Timestamp,
    policy: &TierPolicy,
) -> Result<(), StoreError> {
    repo.put_entry(&new_entry(key, kind, value, now, policy))
}

/// Per-tier numbers of `tokred cache stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierStats {
    /// The tier.
    pub tier: Tier,
    /// Rows in the tier (expired ones included).
    pub entries: u64,
    /// Rows already expired, waiting for `cache prune`.
    pub expired: u64,
    /// Sum of the hits of the rows.
    pub hits: u64,
    /// Total size of the values, in bytes.
    pub bytes: u64,
}

/// Result of `tokred cache stats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStats {
    /// One item per tier, L1 first.
    pub tiers: Vec<TierStats>,
}

impl CacheStats {
    /// Rows in all tiers.
    #[must_use]
    pub fn total_entries(&self) -> u64 {
        self.tiers.iter().map(|t| t.entries).sum()
    }
}

/// Computes the state of the cache per tier at `now`.
pub fn cache_stats(repo: &dyn CacheRepo, now: Timestamp) -> Result<CacheStats, StoreError> {
    let entries = repo.list_entries()?;
    let tiers = Tier::ALL
        .into_iter()
        .map(|tier| {
            let mut stats = TierStats {
                tier,
                entries: 0,
                expired: 0,
                hits: 0,
                bytes: 0,
            };
            for entry in entries.iter().filter(|entry| entry.tier == tier) {
                stats.entries += 1;
                stats.expired += u64::from(is_expired(entry, now));
                stats.hits += u64::from(entry.hits);
                stats.bytes += entry.value.len() as u64;
            }
            stats
        })
        .collect();
    Ok(CacheStats { tiers })
}

/// Removes the expired rows and returns how many were removed. Never call it from a hook.
pub fn cache_prune(repo: &dyn CacheRepo, now: Timestamp) -> Result<usize, StoreError> {
    repo.delete_expired(now)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use proptest::prelude::*;

    use super::*;

    fn policy() -> TierPolicy {
        TierPolicy::default()
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("4h"), Some(14_400));
        assert_eq!(parse_duration(" 24H "), Some(86_400));
        assert_eq!(parse_duration("30m"), Some(1800));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("7d"), Some(604_800));
    }

    #[test]
    fn bad_durations_are_rejected() {
        for bad in [
            "",
            "h",
            "4",
            "0h",
            "-4h",
            "4x",
            "1.5h",
            "4 h",
            "４h",
            "99999999999999999999h",
        ] {
            assert_eq!(parse_duration(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_huge_duration_saturates_instead_of_overflowing() {
        assert_eq!(parse_duration("9223372036854775807d"), Some(i64::MAX));
    }

    #[test]
    fn the_policy_comes_from_the_config_with_defaults_for_typos() {
        let config = CacheConfig {
            l1_ttl: "1h".into(),
            l2_ttl: "oops".into(),
            l2_hits: 2,
            l3_hits: 5,
        };
        assert_eq!(
            TierPolicy::from_config(&config),
            TierPolicy {
                l1_ttl_secs: 3600,
                l2_ttl_secs: 86_400,
                l2_hits: 2,
                l3_hits: 5
            }
        );
        assert_eq!(
            TierPolicy::from_config(&CacheConfig::default()),
            TierPolicy::default()
        );
    }

    #[test]
    fn tiers_round_trip() {
        for tier in Tier::ALL {
            assert_eq!(Tier::from_int(tier.as_int()), Some(tier));
        }
        assert_eq!(Tier::from_int(0), None);
        assert_eq!(Tier::from_int(4), None);
    }

    #[test]
    fn a_new_entry_is_l1_and_expires_after_the_l1_ttl() {
        let entry = new_entry("k", "kind", vec![1], Timestamp(1000), &policy());
        assert_eq!(entry.tier, Tier::L1);
        assert_eq!(entry.hits, 0);
        assert_eq!(entry.expires_at, Some(Timestamp(1000 + 4 * 3600)));
        assert!(!is_expired(&entry, Timestamp(1000 + 4 * 3600 - 1)));
        assert!(is_expired(&entry, Timestamp(1000 + 4 * 3600)));
    }

    #[test]
    fn hits_promote_through_the_tiers() {
        let mut entry = new_entry("k", "kind", vec![], Timestamp(0), &policy());
        let mut tiers = Vec::new();
        for i in 1..=10 {
            entry = record_hit(&entry, Timestamp(i * 10), &policy());
            tiers.push(entry.tier);
        }
        assert_eq!(tiers[0], Tier::L1);
        assert_eq!(tiers[1], Tier::L1);
        assert_eq!(tiers[2], Tier::L2); // third hit
        assert_eq!(tiers[8], Tier::L2);
        assert_eq!(tiers[9], Tier::L3); // tenth hit
        assert_eq!(entry.expires_at, None);
    }

    #[test]
    fn l1_hits_do_not_renew_but_l2_hits_do() {
        let entry = new_entry("k", "kind", vec![], Timestamp(0), &policy());
        let first = record_hit(&entry, Timestamp(100), &policy());
        assert_eq!(first.expires_at, entry.expires_at);
        let mut l2 = first;
        for t in [200, 300] {
            l2 = record_hit(&l2, Timestamp(t), &policy());
        }
        assert_eq!(l2.tier, Tier::L2);
        assert_eq!(l2.expires_at, Some(Timestamp(300 + 24 * 3600)));
        assert_eq!(l2.last_hit, Timestamp(300));
    }

    #[test]
    fn keys_include_kind_subject_and_hash() {
        assert_eq!(content_key("repo_files", "/r", "abc"), "repo_files:abc:/r");
        assert_ne!(
            content_key("repo_files", "/r", "abc"),
            content_key("repo_files", "/r", "abd")
        );
    }

    proptest! {
        #[test]
        fn the_tier_never_goes_down_as_hits_grow(
            l2 in 1u32..50, extra in 0u32..50, hits in 0u32..200,
        ) {
            let policy = TierPolicy { l2_hits: l2, l3_hits: l2 + extra, ..TierPolicy::default() };
            let mut entry = new_entry("k", "t", vec![], Timestamp(0), &policy);
            entry.hits = hits;
            let lower = next_tier(&entry, &policy);
            entry.hits = hits + 1;
            prop_assert!(next_tier(&entry, &policy) >= lower);
        }

        #[test]
        fn the_tier_matches_the_thresholds(
            l2 in 1u32..50, extra in 0u32..50, hits in 0u32..200,
        ) {
            let policy = TierPolicy { l2_hits: l2, l3_hits: l2 + extra, ..TierPolicy::default() };
            let mut entry = new_entry("k", "t", vec![], Timestamp(0), &policy);
            entry.hits = hits;
            let tier = next_tier(&entry, &policy);
            prop_assert_eq!(tier == Tier::L3, hits >= l2 + extra);
            prop_assert_eq!(tier >= Tier::L2, hits >= l2);
        }

        #[test]
        fn only_l3_is_persistent_and_the_others_have_the_documented_expiry(
            created in 0i64..1_000_000, delta in 0i64..1_000_000, hits in 0u32..40,
        ) {
            let policy = TierPolicy::default();
            let now = Timestamp(created + delta);
            let mut entry = new_entry("k", "t", vec![], Timestamp(created), &policy);
            entry.hits = hits;
            let entry = record_hit(&entry, now, &policy);
            match entry.tier {
                Tier::L3 => prop_assert_eq!(entry.expires_at, None),
                Tier::L2 => prop_assert_eq!(
                    entry.expires_at,
                    Some(Timestamp(now.0 + policy.l2_ttl_secs))
                ),
                Tier::L1 => prop_assert_eq!(
                    entry.expires_at,
                    Some(Timestamp(created + policy.l1_ttl_secs))
                ),
            }
            prop_assert_eq!(
                is_expired(&entry, now),
                entry.tier == Tier::L1 && delta >= policy.l1_ttl_secs
            );
        }

        #[test]
        fn durations_are_amount_times_unit(amount in 1i64..100_000, unit in 0usize..4) {
            let (suffix, factor) = [("s", 1), ("m", 60), ("h", 3600), ("d", 86_400)][unit];
            prop_assert_eq!(parse_duration(&format!("{amount}{suffix}")), Some(amount * factor));
        }
    }

    #[derive(Default)]
    struct MemRepo {
        rows: RefCell<Vec<CacheEntry>>,
    }

    impl CacheRepo for MemRepo {
        fn get_entry(&self, key: &str) -> Result<Option<CacheEntry>, StoreError> {
            Ok(self.rows.borrow().iter().find(|e| e.key == key).cloned())
        }
        fn put_entry(&self, entry: &CacheEntry) -> Result<(), StoreError> {
            let mut rows = self.rows.borrow_mut();
            rows.retain(|e| e.key != entry.key);
            rows.push(entry.clone());
            Ok(())
        }
        fn delete_expired(&self, now: Timestamp) -> Result<usize, StoreError> {
            let mut rows = self.rows.borrow_mut();
            let before = rows.len();
            rows.retain(|e| !is_expired(e, now));
            Ok(before - rows.len())
        }
        fn list_entries(&self) -> Result<Vec<CacheEntry>, StoreError> {
            Ok(self.rows.borrow().clone())
        }
    }

    #[test]
    fn get_counts_hits_and_promotes_and_expired_entries_miss() {
        let repo = MemRepo::default();
        let p = policy();
        assert_eq!(cache_get(&repo, "k", Timestamp(0), &p).unwrap(), None);
        cache_put(&repo, "k", "kind", b"v".to_vec(), Timestamp(0), &p).unwrap();
        for i in 1..=3 {
            let hit = cache_get(&repo, "k", Timestamp(i), &p).unwrap().unwrap();
            assert_eq!(hit.hits, u32::try_from(i).unwrap());
        }
        assert_eq!(repo.get_entry("k").unwrap().unwrap().tier, Tier::L2);

        cache_put(&repo, "old", "kind", vec![], Timestamp(0), &p).unwrap();
        let late = Timestamp(5 * 3600);
        assert_eq!(cache_get(&repo, "old", late, &p).unwrap(), None);
        assert_eq!(cache_peek(&repo, "old", late).unwrap(), None);
        assert_eq!(repo.get_entry("old").unwrap().unwrap().hits, 0);
    }

    #[test]
    fn peek_does_not_count_a_hit() {
        let repo = MemRepo::default();
        cache_put(&repo, "k", "kind", vec![], Timestamp(0), &policy()).unwrap();
        assert!(cache_peek(&repo, "k", Timestamp(1)).unwrap().is_some());
        assert_eq!(repo.get_entry("k").unwrap().unwrap().hits, 0);
        assert_eq!(cache_peek(&repo, "missing", Timestamp(1)).unwrap(), None);
    }

    #[test]
    fn stats_and_prune() {
        let repo = MemRepo::default();
        let p = policy();
        cache_put(&repo, "a", "kind", vec![0; 10], Timestamp(0), &p).unwrap();
        cache_put(&repo, "b", "kind", vec![0; 5], Timestamp(10_000), &p).unwrap();
        for t in 1..=3 {
            cache_get(&repo, "b", Timestamp(10_000 + t), &p).unwrap();
        }
        let now = Timestamp(5 * 3600);
        let stats = cache_stats(&repo, now).unwrap();
        assert_eq!(stats.total_entries(), 2);
        assert_eq!(
            stats.tiers[0],
            TierStats {
                tier: Tier::L1,
                entries: 1,
                expired: 1,
                hits: 0,
                bytes: 10
            }
        );
        assert_eq!(
            stats.tiers[1],
            TierStats {
                tier: Tier::L2,
                entries: 1,
                expired: 0,
                hits: 3,
                bytes: 5
            }
        );
        assert_eq!(stats.tiers[2].entries, 0);

        assert_eq!(cache_prune(&repo, now).unwrap(), 1);
        assert_eq!(cache_stats(&repo, now).unwrap().total_entries(), 1);
        assert_eq!(cache_prune(&repo, now).unwrap(), 0);
    }
}
