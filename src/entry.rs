use std::fmt::Display;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultsEntry {
    pub message_link: String,
    pub poster: String,
    pub message_contents: String,
    pub bracket: Option<String>,
    pub player1_id: Option<u64>,
    pub player1_name: Option<String>,
    pub player1_score: Option<i32>,
    pub player2_id: Option<u64>,
    pub player2_name: Option<String>,
    pub player2_score: Option<i32>,
    pub map_draft: Option<String>,
    pub civ_draft: Option<String>,
    pub replays_link: Option<String>,
}

pub const SHEET_COLUMN_COUNT: usize = 14;

impl ResultsEntry {
    pub fn new(message_link: String, poster: String, message_contents: String) -> Self {
        Self {
            message_link,
            poster,
            message_contents,
            ..Self::default()
        }
    }

    pub fn get_row(&self) -> Vec<String> {
        vec![
            self.message_link.clone(),
            self.poster.clone(),
            optstr(self.bracket.as_deref()),
            optstr(self.player1_id),
            optstr(self.player1_name.as_deref()),
            optstr(self.player1_score),
            optstr(self.player2_id),
            optstr(self.player2_name.as_deref()),
            optstr(self.player2_score),
            optstr(self.map_draft.as_deref()),
            optstr(self.civ_draft.as_deref()),
            optstr(self.replays_link.as_deref()),
            self.message_contents.clone(),
        ]
    }
}

fn optstr<T: Display>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_row_all_none_returns_empty_optional_columns() {
        let entry = ResultsEntry::new("link".into(), "poster".into(), "body".into());
        let row = entry.get_row();
        assert_eq!(row.len(), SHEET_COLUMN_COUNT - 1);
        assert_eq!(row[0], "link");
        assert_eq!(row[1], "poster");
        for cell in &row[2..12] {
            assert_eq!(cell, "");
        }
        assert_eq!(row[12], "body");
    }

    #[test]
    fn get_row_score_zero_renders_as_zero_not_blank() {
        let mut entry = ResultsEntry::new("".into(), "".into(), "".into());
        entry.player1_score = Some(0);
        entry.player2_score = Some(3);
        let row = entry.get_row();
        assert_eq!(row[5], "0");
        assert_eq!(row[8], "3");
    }

    #[test]
    fn get_row_fully_populated() {
        let entry = ResultsEntry {
            message_link: "https://discord.com/m/1".into(),
            poster: "Alice".into(),
            message_contents: "raw".into(),
            bracket: Some("Recruit SF".into()),
            player1_id: Some(111),
            player1_name: Some("Alice".into()),
            player1_score: Some(2),
            player2_id: Some(222),
            player2_name: Some("Bob".into()),
            player2_score: Some(1),
            map_draft: Some("https://maps".into()),
            civ_draft: Some("https://civs".into()),
            replays_link: Some("gcs://b/x".into()),
        };
        assert_eq!(
            entry.get_row(),
            vec![
                "https://discord.com/m/1",
                "Alice",
                "Recruit SF",
                "111",
                "Alice",
                "2",
                "222",
                "Bob",
                "1",
                "https://maps",
                "https://civs",
                "gcs://b/x",
                "raw",
            ]
        );
    }
}
