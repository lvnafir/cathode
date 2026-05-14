use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Config {
    pub region: String,
    pub feed_sample: usize,
    #[serde(default = "default_max_resolution")]
    pub max_resolution: u32,
    #[serde(default = "default_channels")]
    pub channels: Vec<Channel>,
    #[serde(default = "default_searches")]
    pub searches: Vec<String>,
}

pub const QUALITY_OPTIONS: &[(u32, &str)] = &[
    (2160, "2160p (4K)"),
    (1440, "1440p"),
    (1080, "1080p"),
    (720,  "720p"),
    (480,  "480p"),
    (360,  "360p"),
];

fn default_max_resolution() -> u32 {
    1080
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Channel {
    pub name: String,
    pub id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            region: "US".into(),
            feed_sample: 4,
            max_resolution: default_max_resolution(),
            channels: default_channels(),
            searches: default_searches(),
        }
    }
}

fn default_channels() -> Vec<Channel> {
    vec![
        // Science/Education
        Channel { name: "Vsauce".into(), id: "UC6nSFpj9HTCZ5t-N3Rm3-HA".into() },
        Channel { name: "Anton Petrov".into(), id: "UCciQ8wFcVoIIMi-lfu8-cjQ".into() },
        Channel { name: "NileRed".into(), id: "UCFhXFikryT4aFcLkLw2LBLA".into() },
        Channel { name: "Periodic Videos".into(), id: "UCtESv1e7ntJaLJYKIO1FoYw".into() },
        Channel { name: "Computerphile".into(), id: "UC9-y-6csu5WGm29I7JiwpnA".into() },
        Channel { name: "3Blue1Brown".into(), id: "UCYO_jab_esuFRV4b17AJtAw".into() },
        Channel { name: "Sebastian Lague".into(), id: "UCmtyQOKKmrMVaKuRXz02jbQ".into() },
        Channel { name: "Two Minute Papers".into(), id: "UCbfYPyITQ-7l4upoX8nvctg".into() },
        Channel { name: "Adam Savage's Tested".into(), id: "UCiDJtJKMICpb9B1qf7qjEOA".into() },
        Channel { name: "PBS Space Time".into(), id: "UC7_gcs09iThXybpVgjHZ_7g".into() },
        // Tech/Linux
        Channel { name: "Diinki".into(), id: "UCrk2bNxxxLP-Qd77KxBJ3Xg".into() },
        Channel { name: "WindowsG Electronics".into(), id: "UC8EYr_ArKMKaxfgRq-iCKzA".into() },
        Channel { name: "Bread on Penguins".into(), id: "UCwHwDuNd9lCdA7chyyquDXw".into() },
        Channel { name: "LaurieWired".into(), id: "UCJXa3_WNNmIpewOtCHf3B0g".into() },
        // Entertainment
        Channel { name: "PewDiePie".into(), id: "UC-lHJZR3Gqxm24_Vd_AJ5Yw".into() },
        Channel { name: "Markiplier".into(), id: "UC7_YxT-KID8kRbqZo7MyscQ".into() },
        Channel { name: "Jacksepticeye".into(), id: "UCYzPXprvl5Y-Sf0g4vX-m6g".into() },
        Channel { name: "Michael Reeves".into(), id: "UCtHaxi4GTYDpJgMSGy7AeSw".into() },
        Channel { name: "Ice Cream Sandwich".into(), id: "UCOsATJw-IZgqGT8MFrHjKGg".into() },
        Channel { name: "Noodle".into(), id: "UCj74rJ9Lgl3WTngq675wxKg".into() },
        Channel { name: "Jaiden Animations".into(), id: "UCGwu0nbY2wSkW8N-cghnLpA".into() },
        Channel { name: "Berd".into(), id: "UCRei8TBpt4r0WPZ7MkiKmVg".into() },
        // Music
        Channel { name: "Virtual Riot".into(), id: "UCVtJOq_ziepf5MpjsTWxJeg".into() },
        Channel { name: "Eliminate".into(), id: "UCI7kKmUuSQOHUvSWIYFDf1Q".into() },
        Channel { name: "Mr Bill".into(), id: "UCJgBqh3tYway5A5VgV4dZRw".into() },
        Channel { name: "DubstepGutter".into(), id: "UCG6QEHCBfWZOnv7UVxappyw".into() },
        Channel { name: "NIGHTMODE".into(), id: "UCHEAsGPcCU9GKK2jEdhlpIQ".into() },
        Channel { name: "MrSuicideSheep".into(), id: "UC5nc_ZtjKW1htCVZVRxlQAQ".into() },
    ]
}

fn default_searches() -> Vec<String> {
    vec![
        "linux ricing".into(),
        "rust programming".into(),
        "music production tutorial".into(),
        "math explained".into(),
        "space documentary".into(),
    ]
}

impl Config {
    pub fn load() -> Self {
        let path = config_dir().join("config.toml");
        if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&text).unwrap_or_default()
        } else {
            Config::default()
        }
    }

    pub fn save(&self) {
        let path = config_dir().join("config.toml");
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, text);
        }
    }

    pub fn has_channel(&self, channel_id: &str) -> bool {
        self.channels.iter().any(|c| c.id == channel_id)
    }

    pub fn follow(&mut self, name: String, channel_id: String) {
        if !self.has_channel(&channel_id) {
            self.channels.push(Channel { name, id: channel_id });
            self.save();
        }
    }

    pub fn unfollow(&mut self, channel_id: &str) {
        self.channels.retain(|c| c.id != channel_id);
        self.save();
    }

    pub fn subscriptions_path() -> PathBuf {
        config_dir().join("subscriptions.csv")
    }
}

pub fn config_dir() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("cathode");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
