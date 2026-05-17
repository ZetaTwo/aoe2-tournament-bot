use tracing::{debug, warn};

use crate::config::Tournament;

#[derive(Debug, Clone, Copy)]
pub struct MatchInput<'a> {
    pub guild_id: Option<u64>,
    pub channel_name: &'a str,
    pub category: Option<&'a str>,
}

pub fn match_tournament<'a>(
    tournaments: &'a [Tournament],
    input: MatchInput<'_>,
) -> Option<&'a Tournament> {
    let matches: Vec<&Tournament> = tournaments
        .iter()
        .filter(|t| tournament_matches(t, input))
        .collect();

    let non_catch_all: Vec<&&Tournament> = matches.iter().filter(|t| !t.catch_all).collect();
    if non_catch_all.len() > 1 {
        let names: Vec<&str> = non_catch_all.iter().map(|t| t.name.as_str()).collect();
        warn!(
            channel = input.channel_name,
            guild = ?input.guild_id,
            tournaments = ?names,
            "channel matched multiple non-catch-all tournaments; using the first",
        );
    }

    matches.into_iter().next()
}

fn tournament_matches(t: &Tournament, input: MatchInput<'_>) -> bool {
    if let Some(want_guild) = t.guild_id {
        if input.guild_id != Some(want_guild) {
            debug!(
                tournament = %t.name,
                want_guild,
                got_guild = ?input.guild_id,
                "rejected: guild_id mismatch",
            );
            return false;
        }
    }
    if let Some(want_cat) = t.category.as_deref() {
        if input.category != Some(want_cat) {
            debug!(
                tournament = %t.name,
                want_category = want_cat,
                got_category = ?input.category,
                "rejected: category mismatch",
            );
            return false;
        }
    }
    if !t.channel_pattern.is_match(input.channel_name) {
        debug!(
            tournament = %t.name,
            pattern = %t.channel_pattern,
            channel = input.channel_name,
            "rejected: channel_pattern did not match",
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn t(name: &str, guild: Option<u64>, pat: &str, catch_all: bool) -> Tournament {
        tc(name, guild, None, pat, catch_all)
    }

    fn tc(
        name: &str,
        guild: Option<u64>,
        category: Option<&str>,
        pat: &str,
        catch_all: bool,
    ) -> Tournament {
        Tournament {
            name: name.into(),
            guild_id: guild,
            category: category.map(str::to_string),
            channel_pattern: Regex::new(pat).unwrap(),
            catch_all,
            sheet_tab: name.into(),
            gcs_prefix: format!("{}/", name.to_lowercase()),
        }
    }

    fn input<'a>(guild: Option<u64>, channel: &'a str) -> MatchInput<'a> {
        MatchInput {
            guild_id: guild,
            channel_name: channel,
            category: None,
        }
    }

    fn input_cat<'a>(guild: Option<u64>, channel: &'a str, category: &'a str) -> MatchInput<'a> {
        MatchInput {
            guild_id: guild,
            channel_name: channel,
            category: Some(category),
        }
    }

    #[test]
    fn matches_specific_tournament_by_guild_and_channel() {
        let tournaments = vec![
            t("SF", Some(100), "^sf-.*-results$", false),
            t("TG", Some(100), "^tg-.*-results$", false),
        ];
        let m = match_tournament(&tournaments, input(Some(100), "sf-final-results")).unwrap();
        assert_eq!(m.name, "SF");
    }

    #[test]
    fn no_match_returns_none_without_catch_all() {
        let tournaments = vec![t("SF", Some(100), "^sf-.*-results$", false)];
        assert!(match_tournament(&tournaments, input(Some(100), "general")).is_none());
    }

    #[test]
    fn catch_all_wins_when_no_specific_match() {
        let tournaments = vec![
            t("SF", Some(100), "^sf-.*-results$", false),
            t("Unknown", None, "^.*results.*$", true),
        ];
        let m = match_tournament(&tournaments, input(Some(999), "team-results")).unwrap();
        assert_eq!(m.name, "Unknown");
    }

    #[test]
    fn specific_match_takes_priority_over_catch_all() {
        let tournaments = vec![
            t("SF", Some(100), "^sf-.*-results$", false),
            t("Unknown", None, "^.*results.*$", true),
        ];
        let m = match_tournament(&tournaments, input(Some(100), "sf-r1-results")).unwrap();
        assert_eq!(m.name, "SF");
    }

    #[test]
    fn guild_filter_excludes_wrong_guild() {
        let tournaments = vec![t("SF", Some(100), "^sf-.*-results$", false)];
        assert!(match_tournament(&tournaments, input(Some(200), "sf-x-results")).is_none());
    }

    #[test]
    fn first_match_wins_among_overlapping_specific() {
        let tournaments = vec![
            t("SF-A", Some(100), "^sf-.*$", false),
            t("SF-B", Some(100), "^sf-.*$", false),
        ];
        let m = match_tournament(&tournaments, input(Some(100), "sf-foo")).unwrap();
        assert_eq!(m.name, "SF-A");
    }

    #[test]
    fn non_results_channel_in_configured_guild_does_not_match() {
        let tournaments = vec![t("SF", Some(100), "^.*results.*$", false)];
        assert!(match_tournament(&tournaments, input(Some(100), "general")).is_none());
    }

    #[test]
    fn category_constraint_matches_when_equal() {
        let tournaments = vec![tc(
            "SF",
            Some(100),
            Some("Recruit SF Bracket"),
            "^.*results.*$",
            false,
        )];
        let m = match_tournament(
            &tournaments,
            input_cat(Some(100), "r1-results", "Recruit SF Bracket"),
        )
        .unwrap();
        assert_eq!(m.name, "SF");
    }

    #[test]
    fn category_constraint_rejects_different_category() {
        let tournaments = vec![tc(
            "SF",
            Some(100),
            Some("Recruit SF Bracket"),
            "^.*results.*$",
            false,
        )];
        assert!(match_tournament(
            &tournaments,
            input_cat(Some(100), "r1-results", "General SF Bracket")
        )
        .is_none());
    }

    #[test]
    fn category_constraint_rejects_missing_category() {
        let tournaments = vec![tc(
            "SF",
            Some(100),
            Some("Recruit SF Bracket"),
            "^.*results.*$",
            false,
        )];
        assert!(match_tournament(&tournaments, input(Some(100), "r1-results")).is_none());
    }

    #[test]
    fn unset_category_matches_regardless_of_message_category() {
        let tournaments = vec![t("SF", Some(100), "^.*results.*$", false)];
        let m = match_tournament(
            &tournaments,
            input_cat(Some(100), "r1-results", "Whatever Bracket"),
        )
        .unwrap();
        assert_eq!(m.name, "SF");
    }
}
