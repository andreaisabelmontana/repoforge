//! Rubric configuration. Sensible defaults are baked in; a `repoforge.toml` can override any
//! weight or threshold so teams can tune the bar without recompiling.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub weights: Weights,
    pub thresholds: Thresholds,
    pub readme_min_chars: usize,
    pub min_topics: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            weights: Weights::default(),
            thresholds: Thresholds::default(),
            readme_min_chars: 800,
            min_topics: 3,
        }
    }
}

/// Per-check weights. They need not sum to 100 — the score is normalised against their total.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Weights {
    pub description: u32,
    pub topics: u32,
    pub readme: u32,
    pub readme_depth: u32,
    pub license: u32,
    pub gitignore: u32,
    pub ci: u32,
    pub tests: u32,
    pub homepage: u32,
    pub activity: u32,
    pub structure: u32,
}

impl Default for Weights {
    fn default() -> Self {
        // Tuned to sum to 100 so a raw score reads like a percentage out of the box.
        Self {
            description: 8,
            topics: 7,
            readme: 12,
            readme_depth: 13,
            license: 12,
            gitignore: 6,
            ci: 12,
            tests: 10,
            homepage: 5,
            activity: 8,
            structure: 7,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            a: 90,
            b: 80,
            c: 70,
            d: 60,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
    }

    /// Load `repoforge.toml` from the current directory if it exists, else defaults.
    pub fn load_or_default(explicit: Option<&Path>) -> Result<Config> {
        if let Some(p) = explicit {
            return Config::load(p);
        }
        let default_path = Path::new("repoforge.toml");
        if default_path.exists() {
            Config::load(default_path)
        } else {
            Ok(Config::default())
        }
    }
}
