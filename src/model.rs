//! Online logistic regression for click prediction.
//!
//! A simple linear model that learns which title terms predict watches.
//! Persists weights to disk as JSON. Updates online after each watch/skip.
//!
//! Features: TF-IDF tokens from video titles.
//! Label: 1.0 = watched, 0.0 = shown but not clicked.
//!
//! The model learns interactions implicitly through weight co-occurrence:
//! if "rust" and "programming" both have high weights, videos containing
//! both get boosted multiplicatively via the sigmoid.

use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const LEARNING_RATE: f64 = 0.05;
const L2_LAMBDA: f64 = 0.001; // regularization to prevent overfitting on small data

#[derive(Serialize, Deserialize)]
pub struct ClickModel {
    weights: HashMap<String, f64>,
    bias: f64,
    examples_seen: u64,
}

impl ClickModel {
    pub fn load() -> Self {
        let path = config::config_dir().join("model.json");
        if path.exists() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(model) = serde_json::from_str(&text) {
                    return model;
                }
            }
        }
        ClickModel {
            weights: HashMap::new(),
            bias: 0.0,
            examples_seen: 0,
        }
    }

    fn save(&self) {
        let path = config::config_dir().join("model.json");
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(&path, text);
        }
    }

    /// Predict P(click) for a video title. Returns 0.0 to 1.0.
    pub fn predict(&self, title: &str) -> f64 {
        let tokens = tokenize(title);
        let mut z = self.bias;
        for token in &tokens {
            z += self.weights.get(token).copied().unwrap_or(0.0);
        }
        sigmoid(z)
    }

    /// Update on a positive example (user watched this video).
    pub fn train_positive(&mut self, title: &str) {
        self.update(title, 1.0);
    }

    /// Update on a weak negative example (video was shown but not clicked).
    /// Uses a reduced learning rate since skips are noisy.
    pub fn train_negative(&mut self, title: &str) {
        self.update_with_rate(title, 0.0, LEARNING_RATE * 0.3);
    }

    fn update(&mut self, title: &str, label: f64) {
        self.update_with_rate(title, label, LEARNING_RATE);
    }

    fn update_with_rate(&mut self, title: &str, label: f64, lr: f64) {
        let tokens = tokenize(title);
        if tokens.is_empty() {
            return;
        }

        let pred = self.predict(title);
        let error = label - pred; // gradient of log-loss

        // SGD update with L2 regularization
        self.bias += lr * error;

        for token in &tokens {
            let w = self.weights.entry(token.clone()).or_insert(0.0);
            *w += lr * error - L2_LAMBDA * *w;
        }

        self.examples_seen += 1;

        // Save every 10 updates
        if self.examples_seen % 10 == 0 {
            self.save();
        }
    }

    /// Force save (call on app exit or after a watch).
    pub fn flush(&self) {
        self.save();
    }

    pub fn is_trained(&self) -> bool {
        self.examples_seen > 0
    }

    /// Export weights for use across thread boundaries.
    pub fn export_weights(&self) -> (HashMap<String, f64>, f64, bool) {
        (self.weights.clone(), self.bias, self.is_trained())
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() >= 3)
        .collect()
}
