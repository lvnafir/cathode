use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Watch log ──

#[derive(Serialize, Deserialize, Clone)]
pub struct WatchEntry {
    pub video_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub title: String,
    pub query: String,
    pub timestamp: u64,
    pub duration_secs: u64,
}

pub struct History {
    entries: Vec<WatchEntry>,
    impressions: HashMap<String, u32>, // channel_id -> times shown in feed
    path: PathBuf,
}

impl History {
    pub fn load() -> Self {
        let path = config::config_dir().join("history.json");
        let entries = if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        } else {
            Vec::new()
        };
        History {
            entries,
            impressions: HashMap::new(),
            path,
        }
    }

    pub fn log_watch(
        &mut self,
        video_id: &str,
        channel_id: &str,
        channel_name: &str,
        title: &str,
        query: &str,
        duration_secs: u64,
    ) {
        let entry = WatchEntry {
            video_id: video_id.to_string(),
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            title: title.to_string(),
            query: query.to_string(),
            timestamp: now(),
            duration_secs,
        };

        if let Ok(line) = serde_json::to_string(&entry) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                let _ = writeln!(f, "{}", line);
            }
        }

        self.entries.push(entry);
    }

    /// Record that these channels were shown in a feed (for skip tracking).
    pub fn log_impressions(&mut self, channel_ids: &[String]) {
        for id in channel_ids {
            if !id.is_empty() {
                *self.impressions.entry(id.clone()).or_insert(0) += 1;
            }
        }
    }

    pub fn watched_channels(&self) -> HashSet<String> {
        self.entries.iter().map(|e| e.channel_id.clone()).collect()
    }

    // ── Channel affinity (existing, kept as one scoring input) ──

    pub fn channel_affinity(&self) -> HashMap<String, f64> {
        let current = now();
        let mut scores: HashMap<String, f64> = HashMap::new();

        for entry in &self.entries {
            if entry.channel_id.is_empty() {
                continue;
            }

            let age_days = (current.saturating_sub(entry.timestamp)) as f64 / 86400.0;
            let decay = 0.995_f64.powf(age_days);
            let duration_bonus = (entry.duration_secs as f64 / 1800.0).min(1.0);
            let weight = (1.0 + duration_bonus) * decay;

            *scores.entry(entry.channel_id.clone()).or_insert(0.0) += weight;
        }

        scores
    }

    // ── TF-IDF user profile ──

    /// Build a TF-IDF profile from watched video titles.
    /// Returns (user_profile_vector, idf_table) for scoring candidates.
    pub fn tfidf_profile(&self) -> TfidfProfile {
        let current = now();

        // Tokenize all watched titles into term sets
        let docs: Vec<(Vec<String>, f64)> = self
            .entries
            .iter()
            .map(|e| {
                let age_days = (current.saturating_sub(e.timestamp)) as f64 / 86400.0;
                let decay = 0.995_f64.powf(age_days);
                let duration_bonus = (e.duration_secs as f64 / 1800.0).min(1.0);
                let weight = (1.0 + duration_bonus) * decay;
                (tokenize(&e.title), weight)
            })
            .collect();

        let n_docs = docs.len() as f64;
        if n_docs == 0.0 {
            return TfidfProfile {
                profile: HashMap::new(),
                idf: HashMap::new(),
            };
        }

        // Document frequency: how many docs contain each term
        let mut df: HashMap<String, u32> = HashMap::new();
        for (terms, _) in &docs {
            let unique: HashSet<&String> = terms.iter().collect();
            for term in unique {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
        }

        // IDF: log(N / df)
        let idf: HashMap<String, f64> = df
            .iter()
            .map(|(term, count)| {
                (term.clone(), (n_docs / *count as f64).ln().max(0.1))
            })
            .collect();

        // Weighted centroid: average TF-IDF vectors weighted by recency+duration
        let mut profile: HashMap<String, f64> = HashMap::new();
        let mut total_weight = 0.0;

        for (terms, weight) in &docs {
            // TF for this document
            let mut tf: HashMap<&String, u32> = HashMap::new();
            for term in terms {
                *tf.entry(term).or_insert(0) += 1;
            }
            let max_tf = tf.values().copied().max().unwrap_or(1) as f64;

            for (term, count) in &tf {
                let tf_norm = *count as f64 / max_tf;
                let tfidf = tf_norm * idf.get(*term).copied().unwrap_or(0.0);
                *profile.entry((*term).clone()).or_insert(0.0) += tfidf * weight;
            }
            total_weight += weight;
        }

        // Normalize by total weight
        if total_weight > 0.0 {
            for v in profile.values_mut() {
                *v /= total_weight;
            }
        }

        TfidfProfile { profile, idf }
    }

    // ── UCB1 bandit ──

    /// Compute UCB1 scores for each channel.
    /// Reward = watches / impressions. Exploration bonus for under-shown channels.
    pub fn ucb1_scores(&self) -> HashMap<String, f64> {
        let current = now();
        let mut watches: HashMap<String, f64> = HashMap::new();
        let mut watch_counts: HashMap<String, u32> = HashMap::new();

        for entry in &self.entries {
            if entry.channel_id.is_empty() {
                continue;
            }
            let age_days = (current.saturating_sub(entry.timestamp)) as f64 / 86400.0;
            let decay = 0.995_f64.powf(age_days);
            *watches.entry(entry.channel_id.clone()).or_insert(0.0) += decay;
            *watch_counts.entry(entry.channel_id.clone()).or_insert(0) += 1;
        }

        let total_impressions: u32 = self.impressions.values().sum();
        if total_impressions == 0 {
            return watches; // Fall back to raw affinity if no impression data yet
        }

        let ln_total = (total_impressions as f64).ln();
        let c = 1.5; // exploration coefficient

        let mut scores: HashMap<String, f64> = HashMap::new();

        // Score all channels that have either watches or impressions
        let all_channels: HashSet<&String> = watches
            .keys()
            .chain(self.impressions.keys())
            .collect();

        for ch in all_channels {
            let pulls = self.impressions.get(ch).copied().unwrap_or(1) as f64;
            let reward = watches.get(ch).copied().unwrap_or(0.0);
            let mean_reward = reward / pulls;
            let exploration = c * (ln_total / pulls).sqrt();
            scores.insert(ch.clone(), mean_reward + exploration);
        }

        scores
    }

    // ── Serendipity ──

    /// Build a serendipity search query from TF-IDF top terms + lateral words.
    pub fn serendipity_query(&self, tfidf: &TfidfProfile, rng: &mut u64) -> Option<String> {
        if tfidf.profile.is_empty() {
            return None;
        }

        // Top 20 terms from profile
        let mut top_terms: Vec<_> = tfidf.profile.iter().collect();
        top_terms.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_terms: Vec<&str> = top_terms.iter().take(20).map(|(k, _)| k.as_str()).collect();

        if top_terms.is_empty() {
            return None;
        }

        const LATERAL: &[&str] = &[
            "history", "science", "explained", "philosophy", "design",
            "engineering", "documentary", "theory", "deep dive", "breakdown",
            "how it works", "behind the scenes", "beginner guide",
        ];

        *rng = xorshift(*rng);
        let term = top_terms[(*rng as usize) % top_terms.len()];
        *rng = xorshift(*rng);
        let lateral = LATERAL[(*rng as usize) % LATERAL.len()];

        Some(format!("{} {}", term, lateral))
    }
}

// ── TF-IDF types ──

pub struct TfidfProfile {
    pub profile: HashMap<String, f64>,
    pub idf: HashMap<String, f64>,
}

impl TfidfProfile {
    /// Score a candidate video title against the user profile.
    /// Returns cosine similarity (0.0 to 1.0).
    pub fn score_title(&self, title: &str) -> f64 {
        if self.profile.is_empty() {
            return 0.0;
        }

        let terms = tokenize(title);
        if terms.is_empty() {
            return 0.0;
        }

        // Build TF-IDF vector for candidate
        let mut tf: HashMap<&String, u32> = HashMap::new();
        for term in &terms {
            *tf.entry(term).or_insert(0) += 1;
        }
        let max_tf = tf.values().copied().max().unwrap_or(1) as f64;

        let mut candidate: HashMap<String, f64> = HashMap::new();
        for (term, count) in &tf {
            let tf_norm = *count as f64 / max_tf;
            let idf = self.idf.get(*term).copied().unwrap_or(0.1);
            candidate.insert((*term).clone(), tf_norm * idf);
        }

        // Cosine similarity
        let mut dot = 0.0;
        let mut mag_a = 0.0;
        let mut mag_b = 0.0;

        for (term, val) in &self.profile {
            mag_a += val * val;
            if let Some(cval) = candidate.get(term) {
                dot += val * cval;
            }
        }
        for val in candidate.values() {
            mag_b += val * val;
        }

        let denom = mag_a.sqrt() * mag_b.sqrt();
        if denom > 0.0 {
            dot / denom
        } else {
            0.0
        }
    }
}

// ── Shared utilities ──

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3 && !is_stopword(w))
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn xorshift(mut x: u64) -> u64 {
    if x == 0 {
        x = 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn is_stopword(w: &str) -> bool {
    matches!(
        w,
        "the" | "and" | "for" | "are" | "but" | "not" | "you" | "all"
            | "can" | "had" | "her" | "was" | "one" | "our" | "out"
            | "has" | "his" | "how" | "its" | "may" | "new" | "now"
            | "old" | "see" | "way" | "who" | "did" | "get" | "got"
            | "let" | "say" | "she" | "too" | "use" | "with" | "this"
            | "that" | "from" | "they" | "been" | "have" | "many"
            | "some" | "them" | "than" | "what" | "when" | "will"
            | "more" | "make" | "like" | "just" | "over" | "such"
            | "take" | "into" | "most" | "very" | "your" | "also"
            | "about" | "would" | "there" | "their" | "which" | "could"
            | "other" | "were" | "then" | "after" | "should"
            | "video" | "official" | "music" | "full" | "best"
            | "every" | "first" | "episode" | "part" | "still"
    )
}
