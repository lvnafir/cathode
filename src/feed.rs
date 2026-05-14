use crate::api::{Video, YouTubeSource};
use crate::config::Config;
use crate::history::TfidfProfile;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Subscription {
    pub channel_id: String,
}

pub fn load_subscriptions() -> Vec<Subscription> {
    let path = Config::subscriptions_path();
    if !path.exists() {
        return Vec::new();
    }
    parse_csv(&path).unwrap_or_default()
}

fn parse_csv(path: &Path) -> Result<Vec<Subscription>, Box<dyn Error>> {
    let mut rdr = csv::Reader::from_path(path)?;
    let mut subs = Vec::new();
    for result in rdr.records() {
        let record = result?;
        if let Some(id) = record.get(0) {
            subs.push(Subscription {
                channel_id: id.to_string(),
            });
        }
    }
    Ok(subs)
}

pub fn fetch_subscription_feed<S: YouTubeSource + Send + Sync + 'static>(
    source: &Arc<S>,
    subs: &[Subscription],
) -> Vec<Video> {
    let handles: Vec<_> = subs
        .iter()
        .map(|sub| {
            let src = Arc::clone(source);
            let id = sub.channel_id.clone();
            thread::spawn(move || src.channel_videos(&id).unwrap_or_default())
        })
        .collect();

    let mut all_videos = Vec::new();
    for handle in handles {
        if let Ok(videos) = handle.join() {
            all_videos.extend(videos);
        }
    }
    all_videos
}

/// Precomputed recommendation data, passed across thread boundary.
/// Lightweight model weights that can cross thread boundaries.
pub struct ModelWeights {
    pub weights: HashMap<String, f64>,
    pub bias: f64,
    pub trained: bool,
}

impl ModelWeights {
    pub fn predict(&self, title: &str) -> f64 {
        if !self.trained {
            return 0.5; // neutral when untrained
        }
        let tokens: Vec<String> = title
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|w| w.len() >= 3)
            .collect();
        let z: f64 = self.bias
            + tokens.iter().map(|t| self.weights.get(t).copied().unwrap_or(0.0)).sum::<f64>();
        1.0 / (1.0 + (-z).exp())
    }
}

pub struct RecommenderData {
    pub affinity: HashMap<String, f64>,
    pub ucb1: HashMap<String, f64>,
    pub tfidf: TfidfProfile,
    pub model: ModelWeights,
    pub serendipity_query: Option<String>,
    pub watched_channels: HashSet<String>,
}

pub struct CuratedFeed {
    pub videos: Vec<Video>,
    pub sampled_channels: Vec<String>,
}

/// Build a feed using composite scoring:
///   0.4 * tfidf_similarity (content match)
///   0.3 * ucb1_score (explore/exploit balance)
///   0.2 * channel_affinity (watch history)
///   0.1 * serendipity bonus
pub fn fetch_curated_feed<S: YouTubeSource + Send + Sync + 'static>(
    source: &Arc<S>,
    config: &Config,
    rec: &RecommenderData,
) -> CuratedFeed {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut rng = seed;

    // UCB1-weighted channel selection
    let weights: Vec<(usize, f64)> = config
        .channels
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let ucb = rec.ucb1.get(&ch.id).copied().unwrap_or(0.0);
            let affinity = rec.affinity.get(&ch.id).copied().unwrap_or(0.0);
            // Blend UCB1 (explore/exploit) with affinity (pure preference)
            (i, 1.0 + 0.6 * ucb + 0.4 * affinity)
        })
        .collect();

    let n_channels = config.feed_sample.min(config.channels.len());
    let selected = weighted_sample(&weights, n_channels, &mut rng);

    let selected_channels: Vec<_> = selected
        .iter()
        .map(|&idx| config.channels[idx].clone())
        .collect();

    // Search queries from config
    let search_query = if !config.searches.is_empty() {
        rng = xorshift(rng);
        let idx = (rng as usize) % config.searches.len();
        Some(config.searches[idx].clone())
    } else {
        None
    };

    // Parallel fetch: channels + config search + serendipity
    let mut handles = Vec::new();

    for ch in &selected_channels {
        let id = ch.id.clone();
        let name = ch.name.clone();
        let src = Arc::clone(source);
        handles.push(thread::spawn(move || {
            match src.channel_videos(&id) {
                Ok(mut videos) => {
                    videos.truncate(3);
                    (videos, Some(name), false)
                }
                Err(_) => (Vec::new(), None, false),
            }
        }));
    }

    if let Some(query) = search_query {
        let src = Arc::clone(source);
        handles.push(thread::spawn(move || {
            match src.search(&query) {
                Ok(mut videos) => {
                    videos.truncate(5);
                    (videos, None, false)
                }
                Err(_) => (Vec::new(), None, false),
            }
        }));
    }

    // Serendipity slot: search with a lateral query, flag as serendipity
    if let Some(ref query) = rec.serendipity_query {
        let src = Arc::clone(source);
        let q = query.clone();
        let watched = rec.watched_channels.clone();
        handles.push(thread::spawn(move || {
            match src.search(&q) {
                Ok(videos) => {
                    // Filter to channels never watched
                    let novel: Vec<Video> = videos
                        .into_iter()
                        .filter(|v| !v.channel_id.is_empty() && !watched.contains(&v.channel_id))
                        .take(2)
                        .collect();
                    (novel, None, true)
                }
                Err(_) => (Vec::new(), None, true),
            }
        }));
    }

    // Collect results
    let mut all_videos = Vec::new();
    let mut sampled_channels = Vec::new();

    for handle in handles {
        if let Ok((videos, name, _is_serendipity)) = handle.join() {
            all_videos.extend(videos);
            if let Some(n) = name {
                sampled_channels.push(n);
            }
        }
    }

    // Score and sort by composite score
    // When the model is trained, it gets weight. Otherwise TF-IDF carries it.
    let model_weight = if rec.model.trained { 0.25 } else { 0.0 };
    let tfidf_weight = if rec.model.trained { 0.25 } else { 0.4 };

    let mut scored: Vec<(f64, Video)> = all_videos
        .into_iter()
        .map(|v| {
            let tfidf_score = rec.tfidf.score_title(&v.title);
            let model_score = rec.model.predict(&v.title); // 0.0 to 1.0
            let ucb = rec.ucb1.get(&v.channel_id).copied().unwrap_or(0.0);
            let affinity = rec.affinity.get(&v.channel_id).copied().unwrap_or(0.0);
            let is_novel = !v.channel_id.is_empty() && !rec.watched_channels.contains(&v.channel_id);
            let novelty_bonus = if is_novel { 1.0 } else { 0.0 };

            let score = tfidf_weight * tfidf_score
                + model_weight * model_score
                + 0.3 * ucb.min(5.0) / 5.0
                + 0.1 * affinity.min(10.0) / 10.0
                + 0.1 * novelty_bonus;

            (score, v)
        })
        .collect();

    // Sort descending by score, then add some shuffle within similar scores
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Light shuffle: swap adjacent items with small probability to avoid rigid ordering
    for i in 0..scored.len().saturating_sub(1) {
        rng = xorshift(rng);
        if rng % 4 == 0 {
            scored.swap(i, i + 1);
        }
    }

    let videos = scored.into_iter().map(|(_, v)| v).collect();

    CuratedFeed {
        videos,
        sampled_channels,
    }
}

fn weighted_sample(weights: &[(usize, f64)], n: usize, rng: &mut u64) -> Vec<usize> {
    let mut pool: Vec<(usize, f64)> = weights.to_vec();
    let mut picked = Vec::with_capacity(n);

    for _ in 0..n {
        if pool.is_empty() {
            break;
        }

        let total: f64 = pool.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            break;
        }

        *rng = xorshift(*rng);
        let dart = (*rng as f64 / u64::MAX as f64) * total;

        let mut cumulative = 0.0;
        let mut chosen = 0;
        for (i, (_, w)) in pool.iter().enumerate() {
            cumulative += w;
            if cumulative >= dart {
                chosen = i;
                break;
            }
        }

        let (idx, _) = pool.remove(chosen);
        picked.push(idx);
    }

    picked
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
