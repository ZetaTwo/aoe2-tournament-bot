use std::path::Path;

use anyhow::{anyhow, Context, Result};
use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawConfig {
    bot: BotConfig,
    gcp: GcpConfig,
    #[serde(default)]
    tournaments: Vec<RawTournament>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BotConfig {
    pub discord_token: String,
    pub admin_user_ids: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GcpConfig {
    pub bucket: String,
    pub sheet_id: String,
}

#[derive(Debug, Deserialize)]
struct RawTournament {
    name: String,
    guild_id: Option<u64>,
    channel_pattern: String,
    #[serde(default)]
    catch_all: bool,
}

#[derive(Debug, Clone)]
pub struct Tournament {
    pub name: String,
    pub guild_id: Option<u64>,
    pub channel_pattern: Regex,
    pub catch_all: bool,
    pub sheet_tab: String,
    pub gcs_prefix: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bot: BotConfig,
    pub gcp: GcpConfig,
    pub tournaments: Vec<Tournament>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw: RawConfig = Figment::new()
            .merge(Toml::file(path))
            .merge(Env::prefixed("AOE2BOT_").split("__"))
            .extract()
            .with_context(|| format!("loading config from {}", path.display()))?;
        validate(raw)
    }
}

fn validate(raw: RawConfig) -> Result<Config> {
    if raw.bot.admin_user_ids.is_empty() {
        return Err(anyhow!("bot.admin_user_ids must not be empty"));
    }

    let mut tournaments = Vec::with_capacity(raw.tournaments.len());
    for (idx, t) in raw.tournaments.iter().enumerate() {
        let is_last = idx == raw.tournaments.len() - 1;
        if t.catch_all && !is_last {
            return Err(anyhow!(
                "tournament '{}' has catch_all = true but is not the last entry",
                t.name
            ));
        }
        let pattern = Regex::new(&t.channel_pattern).with_context(|| {
            format!(
                "invalid channel_pattern for tournament '{}': {}",
                t.name, t.channel_pattern
            )
        })?;
        let gcs_prefix = kebab_case_prefix(&t.name).ok_or_else(|| {
            anyhow!(
                "tournament name '{}' has no ASCII alphanumeric characters; cannot derive gcs_prefix",
                t.name
            )
        })?;
        tournaments.push(Tournament {
            name: t.name.clone(),
            guild_id: t.guild_id,
            channel_pattern: pattern,
            catch_all: t.catch_all,
            sheet_tab: t.name.clone(),
            gcs_prefix,
        });
    }

    for i in 0..tournaments.len() {
        for j in (i + 1)..tournaments.len() {
            if tournaments[i].name == tournaments[j].name {
                return Err(anyhow!(
                    "duplicate tournament name '{}'",
                    tournaments[i].name
                ));
            }
            if tournaments[i].gcs_prefix == tournaments[j].gcs_prefix {
                return Err(anyhow!(
                    "tournaments '{}' and '{}' share derived gcs_prefix '{}'",
                    tournaments[i].name,
                    tournaments[j].name,
                    tournaments[i].gcs_prefix
                ));
            }
        }
    }

    let catch_all_count = tournaments.iter().filter(|t| t.catch_all).count();
    if catch_all_count > 1 {
        return Err(anyhow!(
            "at most one tournament may set catch_all = true (found {})",
            catch_all_count
        ));
    }

    Ok(Config {
        bot: raw.bot,
        gcp: raw.gcp,
        tournaments,
    })
}

pub fn kebab_case_prefix(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len() + 1);
    let mut last_was_dash = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return None;
    }
    out.push('/');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_from_toml(s: &str) -> RawConfig {
        toml::from_str(s).expect("parse toml")
    }

    #[test]
    fn kebab_case_examples() {
        assert_eq!(kebab_case_prefix("SF 2026").as_deref(), Some("sf-2026/"));
        assert_eq!(
            kebab_case_prefix("  General SF  ").as_deref(),
            Some("general-sf/")
        );
        assert_eq!(kebab_case_prefix("Foo/Bar").as_deref(), Some("foo-bar/"));
        assert_eq!(kebab_case_prefix("--A--B--").as_deref(), Some("a-b/"));
        assert_eq!(kebab_case_prefix("!!!"), None);
        assert_eq!(kebab_case_prefix(""), None);
    }

    #[test]
    fn loads_valid_config() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "tok"
admin_user_ids = [1, 2]

[gcp]
bucket = "b"
sheet_id = "s"

[[tournaments]]
name = "SF 2026"
guild_id = 100
channel_pattern = "^sf-.*-results$"

[[tournaments]]
name = "Unknown"
catch_all = true
channel_pattern = ".*"
"#,
        );
        let cfg = validate(raw).unwrap();
        assert_eq!(cfg.bot.admin_user_ids, vec![1, 2]);
        assert_eq!(cfg.tournaments.len(), 2);
        assert_eq!(cfg.tournaments[0].sheet_tab, "SF 2026");
        assert_eq!(cfg.tournaments[0].gcs_prefix, "sf-2026/");
        assert!(!cfg.tournaments[0].catch_all);
        assert!(cfg.tournaments[1].catch_all);
    }

    #[test]
    fn rejects_empty_admin_user_ids() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "tok"
admin_user_ids = []
[gcp]
bucket = "b"
sheet_id = "s"
"#,
        );
        let err = validate(raw).unwrap_err().to_string();
        assert!(err.contains("admin_user_ids"), "{err}");
    }

    #[test]
    fn rejects_invalid_regex() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "t"
admin_user_ids = [1]
[gcp]
bucket = "b"
sheet_id = "s"
[[tournaments]]
name = "A"
channel_pattern = "["
"#,
        );
        assert!(validate(raw).is_err());
    }

    #[test]
    fn rejects_duplicate_names() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "t"
admin_user_ids = [1]
[gcp]
bucket = "b"
sheet_id = "s"
[[tournaments]]
name = "A"
channel_pattern = ".*"
[[tournaments]]
name = "A"
channel_pattern = ".*"
"#,
        );
        let err = validate(raw).unwrap_err().to_string();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn rejects_duplicate_derived_prefix() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "t"
admin_user_ids = [1]
[gcp]
bucket = "b"
sheet_id = "s"
[[tournaments]]
name = "SF 2026"
channel_pattern = ".*"
[[tournaments]]
name = "sf-2026"
channel_pattern = ".*"
"#,
        );
        let err = validate(raw).unwrap_err().to_string();
        assert!(err.contains("gcs_prefix"), "{err}");
    }

    #[test]
    fn rejects_catch_all_not_last() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "t"
admin_user_ids = [1]
[gcp]
bucket = "b"
sheet_id = "s"
[[tournaments]]
name = "A"
catch_all = true
channel_pattern = ".*"
[[tournaments]]
name = "B"
channel_pattern = ".*"
"#,
        );
        let err = validate(raw).unwrap_err().to_string();
        assert!(err.contains("not the last entry"), "{err}");
    }

    #[test]
    fn rejects_name_with_no_alphanumerics() {
        let raw = raw_from_toml(
            r#"
[bot]
discord_token = "t"
admin_user_ids = [1]
[gcp]
bucket = "b"
sheet_id = "s"
[[tournaments]]
name = "!!!"
channel_pattern = ".*"
"#,
        );
        assert!(validate(raw).is_err());
    }
}
