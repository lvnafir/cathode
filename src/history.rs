use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    path: PathBuf,
}

impl History {
    pub fn load() -> Self {
        let path = config::config_dir().join("history.json");
        let entries = if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            // JSON lines format
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        } else {
            Vec::new()
        };
        History { entries, path }
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

        // Append to file
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

    /// Channel affinity scores: higher = more interested.
    /// Every watch is positive. Longer watches are slightly warmer.
    /// Decay is very slow — 0.995^days, ~60% strength after 100 days.
    pub fn channel_affinity(&self) -> HashMap<String, f64> {
        let current = now();
        let mut scores: HashMap<String, f64> = HashMap::new();

        for entry in &self.entries {
            if entry.channel_id.is_empty() {
                continue;
            }

            let age_days = (current.saturating_sub(entry.timestamp)) as f64 / 86400.0;
            let decay = 0.995_f64.powf(age_days);

            // Base: 1.0 per watch. Bonus for longer watches, but capped.
            // 10s watch = 1.0, 5min = 1.3, 30min = 1.7, 1hr = 2.0
            let duration_bonus = (entry.duration_secs as f64 / 1800.0).min(1.0);
            let weight = (1.0 + duration_bonus) * decay;

            *scores.entry(entry.channel_id.clone()).or_insert(0.0) += weight;
        }

        scores
    }

    /// Extract recurring keywords from watched titles.
    /// Returns top N keywords sorted by frequency, excluding stopwords.
    pub fn topic_keywords(&self, n: usize) -> Vec<String> {
        let current = now();
        let mut word_scores: HashMap<String, f64> = HashMap::new();

        for entry in &self.entries {
            let age_days = (current.saturating_sub(entry.timestamp)) as f64 / 86400.0;
            let decay = 0.995_f64.powf(age_days);

            for word in entry.title.split_whitespace() {
                let w = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                if w.len() >= 3 && !is_stopword(&w) {
                    *word_scores.entry(w).or_insert(0.0) += decay;
                }
            }
        }

        let mut sorted: Vec<_> = word_scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.into_iter().take(n).map(|(w, _)| w).collect()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
            | "video" | "official" | "music"
    )
}
