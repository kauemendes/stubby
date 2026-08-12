//! Process configuration, sourced from environment variables set by the chart.
use anyhow::Context;
use std::time::Duration;

/// Dummy image refs (same env names as the webhook) plus the registry
/// re-check interval.
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    pub backend_image: String,
    pub frontend_image: String,
    pub check_interval: Duration,
}

impl ControllerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            backend_image: required("STUBBY_IMAGE_BACKEND")?,
            frontend_image: required("STUBBY_IMAGE_FRONTEND")?,
            check_interval: parse_interval_secs(std::env::var("STUBBY_CHECK_INTERVAL_SECS").ok().as_deref()),
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    let v = std::env::var(key).with_context(|| format!("{key} must be set"))?;
    let v = v.trim().to_string();
    anyhow::ensure!(!v.is_empty(), "{key} must not be empty");
    Ok(v)
}

/// Parse a whole-seconds interval; fall back to 60s on absent/garbage input.
pub fn parse_interval_secs(raw: Option<&str>) -> Duration {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_interval_seconds() {
        assert_eq!(parse_interval_secs(Some("30")), std::time::Duration::from_secs(30));
        assert_eq!(parse_interval_secs(Some("bad")), std::time::Duration::from_secs(60));
        assert_eq!(parse_interval_secs(None), std::time::Duration::from_secs(60));
    }
}
