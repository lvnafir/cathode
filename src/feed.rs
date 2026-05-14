use crate::api::{Video, YouTubeSource};
use crate::config::Config;
use std::collections::HashMap;
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
            thread::spawn(move || {
                src.channel_videos(&id).unwrap_or_default()
            })
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

pub struct CuratedFeed {
    pub videos: Vec<Video>,
    pub sampled_channels: Vec<String>,
}

/// Build a mixed feed weighted by watch history.
///
/// Channel selection: weighted random sampling. Channels you watch more
/// get picked more often, but every channel has a floor so nothing gets
/// starved entirely (discovery stays alive).
///
/// Search queries: mix of configured searches + topic keywords extracted
/// from your watch history titles.
pub fn fetch_curated_feed<S: YouTubeSource + Send + Sync + 'static>(
    source: &Arc<S>,
    config: &Config,
    affinity: &HashMap<String, f64>,
    topic_keywords: &[String],
) -> CuratedFeed {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut rng = seed;

    // Build weighted channel list: affinity score + floor of 1.0
    let weights: Vec<(usize, f64)> = config
        .channels
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let score = affinity.get(&ch.id).copied().unwrap_or(0.0);
            (i, 1.0 + score) // floor of 1.0 so every channel has a chance
        })
        .collect();

    // Weighted random sample without replacement
    let n_channels = config.feed_sample.min(config.channels.len());
    let selected = weighted_sample(&weights, n_channels, &mut rng);

    let selected_channels: Vec<_> = selected
        .iter()
        .map(|&idx| config.channels[idx].clone())
        .collect();

    // Search queries: pick from configured + topic keywords
    let mut all_queries: Vec<String> = config.searches.clone();
    for kw in topic_keywords {
        // Combine nearby keywords into queries
        all_queries.push(kw.clone());
    }

    let search_query = if !all_queries.is_empty() {
        rng = xorshift(rng);
        let idx = (rng as usize) % all_queries.len();
        Some(all_queries[idx].clone())
    } else {
        None
    };

    // Parallel fetch
    let mut handles = Vec::new();

    for ch in &selected_channels {
        let id = ch.id.clone();
        let name = ch.name.clone();
        let src = Arc::clone(source);
        handles.push(thread::spawn(move || {
            match src.channel_videos(&id) {
                Ok(mut videos) => {
                    videos.truncate(3);
                    (videos, Some(name))
                }
                Err(_) => (Vec::new(), None),
            }
        }));
    }

    if let Some(query) = search_query {
        let src = Arc::clone(source);
        handles.push(thread::spawn(move || {
            match src.search(&query) {
                Ok(mut videos) => {
                    videos.truncate(5);
                    (videos, None)
                }
                Err(_) => (Vec::new(), None),
            }
        }));
    }

    // Collect
    let mut all_videos = Vec::new();
    let mut sampled_channels = Vec::new();

    for handle in handles {
        if let Ok((videos, name)) = handle.join() {
            all_videos.extend(videos);
            if let Some(n) = name {
                sampled_channels.push(n);
            }
        }
    }

    // Shuffle the final mix
    for i in (1..all_videos.len()).rev() {
        rng = xorshift(rng);
        let j = (rng as usize) % (i + 1);
        all_videos.swap(i, j);
    }

    CuratedFeed { videos: all_videos, sampled_channels }
}

/// Weighted random sampling without replacement.
/// Uses the "selection by cumulative weight" approach.
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
    if x == 0 { x = 1; }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}
