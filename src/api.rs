use serde::Deserialize;
use std::error::Error;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Video {
    pub title: String,
    pub video_id: String,
    pub author: String,
    pub channel_id: String,
    pub duration_secs: u64,
    pub views: u64,
    pub published_text: String,
    pub thumbnail_url: String,
}

pub trait YouTubeSource: Send + Sync {
    fn trending(&self, region: &str) -> Result<Vec<Video>, Box<dyn Error>>;
    fn search(&self, query: &str) -> Result<Vec<Video>, Box<dyn Error>>;
    fn channel_videos(&self, ucid: &str) -> Result<Vec<Video>, Box<dyn Error>>;
}

pub struct YtdlpSource;

impl YouTubeSource for YtdlpSource {
    fn trending(&self, _region: &str) -> Result<Vec<Video>, Box<dyn Error>> {
        let output = Command::new("yt-dlp")
            .args([
                "--flat-playlist",
                "--dump-json",
                "--no-warnings",
                "ytsearch20:popular music videos 2026",
            ])
            .output()?;
        parse_ytdlp_json(&output.stdout)
    }

    fn search(&self, query: &str) -> Result<Vec<Video>, Box<dyn Error>> {
        let output = Command::new("yt-dlp")
            .args([
                "--flat-playlist",
                "--dump-json",
                "--no-warnings",
                &format!("ytsearch20:{}", query),
            ])
            .output()?;
        parse_ytdlp_json(&output.stdout)
    }

    fn channel_videos(&self, ucid: &str) -> Result<Vec<Video>, Box<dyn Error>> {
        let output = Command::new("yt-dlp")
            .args([
                "--flat-playlist",
                "--dump-json",
                "--no-warnings",
                "--playlist-items",
                "1:20",
                &format!("https://www.youtube.com/channel/{}/videos", ucid),
            ])
            .output()?;
        parse_ytdlp_json(&output.stdout)
    }
}

#[derive(Deserialize)]
struct YtdlpEntry {
    title: Option<String>,
    id: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    channel_id: Option<String>,
    playlist_channel: Option<String>,
    playlist_channel_id: Option<String>,
    duration: Option<f64>,
    view_count: Option<u64>,
}

fn parse_ytdlp_json(stdout: &[u8]) -> Result<Vec<Video>, Box<dyn Error>> {
    let text = String::from_utf8_lossy(stdout);
    let mut videos = Vec::new();
    for line in text.lines() {
        if let Ok(entry) = serde_json::from_str::<YtdlpEntry>(line) {
            let id = entry.id.unwrap_or_default();
            if !id.is_empty() {
                let thumb = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id);
                let author = entry.channel
                    .or(entry.uploader)
                    .or(entry.playlist_channel.clone())
                    .unwrap_or_default();
                let ch_id = entry.channel_id
                    .or(entry.playlist_channel_id)
                    .unwrap_or_default();
                videos.push(Video {
                    title: entry.title.unwrap_or_default(),
                    video_id: id,
                    author,
                    channel_id: ch_id,
                    duration_secs: entry.duration.unwrap_or(0.0) as u64,
                    views: entry.view_count.unwrap_or(0),
                    published_text: String::new(),
                    thumbnail_url: thumb,
                });
            }
        }
    }
    Ok(videos)
}
