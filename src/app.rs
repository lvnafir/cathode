use crate::api::Video;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::sync::mpsc;
use std::thread;

#[derive(PartialEq)]
pub enum AppState {
    Loading,
    Feed,
    Search,
    Quality,
    Channels,
    Error,
}

pub enum FeedKind {
    Trending,
    Subscriptions,
    Curated(Vec<String>),
}

pub struct App {
    pub state: AppState,
    pub videos: Vec<Video>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub search_input: String,
    pub search_cursor: usize,
    pub feed_kind: FeedKind,
    pub status_msg: String,
    pub error_msg: String,
    pub spinner_frame: usize,
    pub tick_count: u64,
    pub thumbnail: Option<StatefulProtocol>,
    pub thumbnail_video_id: String,
    pub picker: Option<Picker>,
    thumb_rx: Option<mpsc::Receiver<(String, StatefulProtocol)>>,
    pub quality_selected: usize,
    pub quality_video_url: String,
    pub quality_video_info: Option<(String, String, String, String)>, // (video_id, channel_id, channel_name, title)
    pub channel_selected: usize,
    pub channel_scroll: usize,
    pub follow_flash: Option<(String, u64)>,
}

impl App {
    pub fn new() -> Self {
        let picker = Picker::from_query_stdio().ok();

        Self {
            state: AppState::Loading,
            videos: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            search_input: String::new(),
            search_cursor: 0,
            feed_kind: FeedKind::Trending,
            status_msg: "loading...".into(),
            error_msg: String::new(),
            spinner_frame: 0,
            tick_count: 0,
            thumbnail: None,
            thumbnail_video_id: String::new(),
            picker,
            thumb_rx: None,
            quality_selected: 0,
            quality_video_url: String::new(),
            quality_video_info: None,
            channel_selected: 0,
            channel_scroll: 0,
            follow_flash: None,
        }
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;
        if self.tick_count % 4 == 0 {
            self.spinner_frame = (self.spinner_frame + 1) % 4;
        }

        // Clear follow flash after ~2 seconds (60 ticks at 30fps)
        if let Some((_, started)) = &self.follow_flash {
            if self.tick_count - started > 60 {
                self.follow_flash = None;
            }
        }

        // Poll for thumbnail
        if let Some(ref rx) = self.thumb_rx {
            if let Ok((vid, proto)) = rx.try_recv() {
                if vid == self.thumbnail_video_id {
                    self.thumbnail = Some(proto);
                }
                self.thumb_rx = None;
            }
        }

        // Request thumbnail for selected video if changed
        if let Some(video) = self.videos.get(self.selected) {
            if video.video_id != self.thumbnail_video_id && self.picker.is_some() {
                let url = video.thumbnail_url.clone();
                let vid = video.video_id.clone();
                self.thumbnail_video_id = vid.clone();
                self.thumbnail = None;
                self.fetch_thumbnail(&url, &vid);
            }
        }
    }

    fn fetch_thumbnail(&mut self, url: &str, video_id: &str) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        if url.is_empty() {
            return;
        }

        let (tx, rx) = mpsc::channel();
        let url = url.to_string();
        let vid = video_id.to_string();
        let picker_clone = picker.clone();

        thread::spawn(move || {
            let Ok(output) = std::process::Command::new("curl")
                .args(["-sfL", "--max-time", "5", &url])
                .output() else {
                return;
            };
            if !output.status.success() || output.stdout.is_empty() {
                return;
            }
            let Ok(img) = image::load_from_memory(&output.stdout) else {
                return;
            };
            let proto = picker_clone.new_resize_protocol(img);
            let _ = tx.send((vid, proto));
        });

        self.thumb_rx = Some(rx);
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if !self.videos.is_empty() && self.selected < self.videos.len() - 1 {
            self.selected += 1;
        }
    }

    pub fn set_videos(&mut self, videos: Vec<Video>) {
        self.videos = videos;
        self.selected = 0;
        self.scroll_offset = 0;
        self.thumbnail = None;
        self.thumbnail_video_id.clear();
        self.state = AppState::Feed;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error_msg = msg;
        self.state = AppState::Error;
    }

    pub fn enter_search(&mut self) {
        self.state = AppState::Search;
        self.search_input.clear();
        self.search_cursor = 0;
    }

    pub fn exit_search(&mut self) {
        self.state = AppState::Feed;
    }

    pub fn search_insert(&mut self, c: char) {
        self.search_input.insert(self.search_cursor, c);
        self.search_cursor += c.len_utf8();
    }

    pub fn search_backspace(&mut self) {
        if self.search_cursor > 0 {
            let prev = self.search_input[..self.search_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.search_input.drain(prev..self.search_cursor);
            self.search_cursor = prev;
        }
    }

    pub fn enter_quality(&mut self, default_res: u32) {
        use crate::config::QUALITY_OPTIONS;
        if let Some(video) = self.videos.get(self.selected).cloned() {
            self.quality_video_url = format!(
                "https://www.youtube.com/watch?v={}",
                video.video_id
            );
            self.quality_video_info = Some((
                video.video_id,
                video.channel_id,
                video.author,
                video.title,
            ));
            self.quality_selected = QUALITY_OPTIONS
                .iter()
                .position(|(h, _)| *h <= default_res)
                .unwrap_or(2);
            self.state = AppState::Quality;
        }
    }

    pub fn quality_up(&mut self) {
        if self.quality_selected > 0 {
            self.quality_selected -= 1;
        }
    }

    pub fn quality_down(&mut self) {
        use crate::config::QUALITY_OPTIONS;
        if self.quality_selected < QUALITY_OPTIONS.len() - 1 {
            self.quality_selected += 1;
        }
    }

    pub fn exit_quality(&mut self) {
        self.state = AppState::Feed;
    }

    pub fn selected_quality(&self) -> u32 {
        use crate::config::QUALITY_OPTIONS;
        QUALITY_OPTIONS[self.quality_selected].0
    }

    pub fn enter_channels(&mut self) {
        self.channel_selected = 0;
        self.channel_scroll = 0;
        self.state = AppState::Channels;
    }

    pub fn exit_channels(&mut self) {
        self.state = AppState::Feed;
    }

    pub fn channel_up(&mut self) {
        if self.channel_selected > 0 {
            self.channel_selected -= 1;
            if self.channel_selected < self.channel_scroll {
                self.channel_scroll = self.channel_selected;
            }
        }
    }

    pub fn channel_down(&mut self, total: usize) {
        if total > 0 && self.channel_selected < total - 1 {
            self.channel_selected += 1;
        }
    }

    pub fn toggle_follow(&mut self, config: &mut crate::config::Config) {
        if let Some(video) = self.videos.get(self.selected) {
            if video.channel_id.is_empty() {
                return;
            }
            if config.has_channel(&video.channel_id) {
                config.unfollow(&video.channel_id);
                self.follow_flash = Some((format!("unfollowed {}", video.author), self.tick_count));
            } else {
                config.follow(video.author.clone(), video.channel_id.clone());
                self.follow_flash = Some((format!("followed {}", video.author), self.tick_count));
            }
        }
    }

    pub fn reinit_picker(&mut self) {
        self.picker = Picker::from_query_stdio().ok();
        self.thumbnail = None;
        self.thumbnail_video_id.clear();
    }
}

pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

pub fn format_views(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
