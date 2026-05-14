use std::sync::LazyLock;

use regex::Regex;
use tracing::info;

use crate::entry::ResultsEntry;

static DISCORD_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<@(\d+)>").unwrap());
static MAP_DRAFT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)maps?(?:\s+draft)?\s*:?\s*([^\s]+)").unwrap());
static CIV_DRAFT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)civs?(?:\s+draft)?\s*:?\s*([^\s]+)").unwrap());
static SCORE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[^\d]*(\d{1,4})[^\d\v]+(\d{1,4})[^\d]*$").unwrap());

pub fn parse_message_content(entry: &mut ResultsEntry, content: &str) {
    let player_ids: Vec<u64> = DISCORD_TAG
        .captures_iter(content)
        .filter_map(|c| c.get(1)?.as_str().parse().ok())
        .collect();
    if player_ids.len() == 2 {
        entry.player1_id = Some(player_ids[0]);
        entry.player2_id = Some(player_ids[1]);
    } else if !player_ids.is_empty() {
        info!(
            "Found {} players in the message, expected 2",
            player_ids.len()
        );
    }

    let content = DISCORD_TAG.replace_all(content, "");

    if let Some(c) = MAP_DRAFT.captures(&content) {
        entry.map_draft = Some(c.get(1).unwrap().as_str().to_string());
    }
    let content = MAP_DRAFT.replace_all(&content, "");

    if let Some(c) = CIV_DRAFT.captures(&content) {
        entry.civ_draft = Some(c.get(1).unwrap().as_str().to_string());
    }
    let content = CIV_DRAFT.replace_all(&content, "");

    if let Some(c) = SCORE_LINE.captures(&content) {
        if let (Ok(s1), Ok(s2)) = (
            c.get(1).unwrap().as_str().parse::<i32>(),
            c.get(2).unwrap().as_str().parse::<i32>(),
        ) {
            entry.player1_score = Some(s1);
            entry.player2_score = Some(s2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(content: &str) -> ResultsEntry {
        let mut entry = ResultsEntry::new(String::new(), String::new(), String::new());
        parse_message_content(&mut entry, content);
        entry
    }

    const TEST_MESSAGE1: &str = "
<@698259349415657522> vs. <@810249574173245501>  Recruit SF
Civs: https://aoe2cm.net/draft/SfNXP
Map: https://aoe2cm.net/draft/zQKpk
";

    #[test]
    fn parses_message1_two_mentions_no_score() {
        let entry = parsed(TEST_MESSAGE1);
        assert_eq!(entry.player1_id, Some(698_259_349_415_657_522));
        assert_eq!(entry.player2_id, Some(810_249_574_173_245_501));
        assert_eq!(
            entry.civ_draft.as_deref(),
            Some("https://aoe2cm.net/draft/SfNXP")
        );
        assert_eq!(
            entry.map_draft.as_deref(),
            Some("https://aoe2cm.net/draft/zQKpk")
        );
        assert_eq!(entry.player1_score, None);
        assert_eq!(entry.player2_score, None);
    }

    const TEST_MESSAGE2: &str = "
<@698259349415657522> 3-0 <@810249574173245501>  Recruit SF
Civs: https://aoe2cm.net/draft/SfNXP
Map: https://aoe2cm.net/draft/zQKpk
";

    #[test]
    fn parses_message2_score_with_dash() {
        let entry = parsed(TEST_MESSAGE2);
        assert_eq!(entry.player1_id, Some(698_259_349_415_657_522));
        assert_eq!(entry.player2_id, Some(810_249_574_173_245_501));
        assert_eq!(
            entry.civ_draft.as_deref(),
            Some("https://aoe2cm.net/draft/SfNXP")
        );
        assert_eq!(
            entry.map_draft.as_deref(),
            Some("https://aoe2cm.net/draft/zQKpk")
        );
        assert_eq!(entry.player1_score, Some(3));
        assert_eq!(entry.player2_score, Some(0));
    }

    const TEST_MESSAGE3: &str = "
<@359062701831618560> ||0:3|| <@271375929702350849>
General SF
Map draft: https://aoe2cm.net/draft/TlCgx
Civ draft: https://aoe2cm.net/draft/vlrcX
";

    #[test]
    fn parses_message3_spoiler_score_and_draft_keywords() {
        let entry = parsed(TEST_MESSAGE3);
        assert_eq!(entry.player1_id, Some(359_062_701_831_618_560));
        assert_eq!(entry.player2_id, Some(271_375_929_702_350_849));
        assert_eq!(
            entry.civ_draft.as_deref(),
            Some("https://aoe2cm.net/draft/vlrcX")
        );
        assert_eq!(
            entry.map_draft.as_deref(),
            Some("https://aoe2cm.net/draft/TlCgx")
        );
        assert_eq!(entry.player1_score, Some(0));
        assert_eq!(entry.player2_score, Some(3));
    }

    #[test]
    fn single_mention_leaves_ids_unset() {
        let entry = parsed("just <@123> playing alone");
        assert_eq!(entry.player1_id, None);
        assert_eq!(entry.player2_id, None);
    }

    #[test]
    fn three_mentions_leaves_ids_unset() {
        let entry = parsed("<@1> vs <@2> vs <@3>");
        assert_eq!(entry.player1_id, None);
        assert_eq!(entry.player2_id, None);
    }

    #[test]
    fn score_line_with_one_number_does_not_match() {
        let entry = parsed("<@1> won <@2>\nonly 5 here\n");
        assert_eq!(entry.player1_score, None);
        assert_eq!(entry.player2_score, None);
    }

    #[test]
    fn map_draft_url_digits_do_not_become_score() {
        let entry = parsed("<@1> vs <@2>\nMap: https://aoe2cm.net/draft/ab12cd34\n");
        assert_eq!(entry.player1_score, None);
        assert_eq!(entry.player2_score, None);
        assert_eq!(
            entry.map_draft.as_deref(),
            Some("https://aoe2cm.net/draft/ab12cd34")
        );
    }
}
