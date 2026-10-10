//! Splits a script into statements the way the `mysql` client does: it knows about
//! quoted strings, comments and `DELIMITER` commands, so a `;` inside a routine body
//! or a string does not end a statement.
//!
//! Positions are UTF-16 code unit offsets, because that is what the code editor uses.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Statement {
    /// The statement without its delimiter, leading comments or trailing whitespace.
    pub text: String,
    pub start: usize,
    pub end: usize,
}

const DEFAULT_DELIMITER: &str = ";";
const DELIMITER_COMMAND: &str = "delimiter";

struct Scanner {
    chars: Vec<char>,
    /// `utf16_at[i]` is the UTF-16 offset of `chars[i]`; one extra entry marks the end.
    utf16_at: Vec<usize>,
}

impl Scanner {
    fn new(sql: &str) -> Self {
        let chars: Vec<char> = sql.chars().collect();
        let mut utf16_at = Vec::with_capacity(chars.len() + 1);
        let mut offset = 0;
        for character in &chars {
            utf16_at.push(offset);
            offset += character.len_utf16();
        }
        utf16_at.push(offset);
        Self { chars, utf16_at }
    }

    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    fn starts_with(&self, index: usize, text: &[char]) -> bool {
        !text.is_empty() && self.chars.get(index..index + text.len()) == Some(text)
    }

    fn matches_ignoring_case(&self, index: usize, word: &str) -> bool {
        word.chars().enumerate().all(|(offset, expected)| {
            self.at(index + offset)
                .is_some_and(|c| c.eq_ignore_ascii_case(&expected))
        })
    }

    /// `-- ` (the dashes must be followed by whitespace or the end) and `#` comments.
    fn is_line_comment(&self, index: usize) -> bool {
        match self.at(index) {
            Some('#') => true,
            Some('-') => {
                self.at(index + 1) == Some('-')
                    && self
                        .at(index + 2)
                        .is_none_or(|c| c.is_whitespace() || c.is_control())
            }
            _ => false,
        }
    }

    fn end_of_line(&self, mut index: usize) -> usize {
        while self.at(index).is_some_and(|c| c != '\n') {
            index += 1;
        }
        index
    }

    /// Index just past a `/* … */` comment that starts at `index`; the end of input if unterminated.
    fn end_of_block_comment(&self, index: usize) -> usize {
        let mut cursor = index + 2;
        while cursor < self.chars.len() {
            if self.chars[cursor] == '*' && self.at(cursor + 1) == Some('/') {
                return cursor + 2;
            }
            cursor += 1;
        }
        self.chars.len()
    }

    /// Index just past the quoted string or identifier that starts at `index`.
    /// A quote is escaped by doubling it; strings also accept backslash escapes.
    fn end_of_quoted(&self, index: usize) -> usize {
        let quote = self.chars[index];
        let mut cursor = index + 1;
        while cursor < self.chars.len() {
            let current = self.chars[cursor];
            let escaped = current == '\\' && quote != '`';
            let doubled = current == quote && self.at(cursor + 1) == Some(quote);
            if escaped || doubled {
                cursor += 2;
            } else if current == quote {
                return cursor + 1;
            } else {
                cursor += 1;
            }
        }
        self.chars.len()
    }

    /// If a `DELIMITER xx` command starts at `index`, returns the new delimiter and the index after its line.
    fn delimiter_command(&self, index: usize) -> Option<(Vec<char>, usize)> {
        if !self.matches_ignoring_case(index, DELIMITER_COMMAND) {
            return None;
        }
        let after_keyword = index + DELIMITER_COMMAND.len();
        if !self
            .at(after_keyword)
            .is_some_and(|c| c == ' ' || c == '\t')
        {
            return None;
        }
        let line_end = self.end_of_line(after_keyword);
        let token: Vec<char> = self.chars[after_keyword..line_end]
            .iter()
            .collect::<String>()
            .trim()
            .chars()
            .collect();
        (!token.is_empty()).then_some((token, line_end))
    }

    fn statement(&self, start: usize, end: usize) -> Statement {
        Statement {
            text: self.chars[start..end].iter().collect(),
            start: self.utf16_at[start],
            end: self.utf16_at[end],
        }
    }
}

pub fn split_statements(sql: &str) -> Vec<Statement> {
    let scanner = Scanner::new(sql);
    let mut delimiter: Vec<char> = DEFAULT_DELIMITER.chars().collect();
    let mut statements = Vec::new();
    let mut start: Option<usize> = None;
    // Index just past the last character that belongs to the statement (not trailing space or comments).
    let mut significant_end = 0;
    let mut index = 0;

    while index < scanner.chars.len() {
        let current = scanner.chars[index];

        let Some(begun_at) = start else {
            // Between statements: skip whitespace and comments, handle DELIMITER, find the first real character.
            if current.is_whitespace() {
                index += 1;
            } else if scanner.is_line_comment(index) {
                index = scanner.end_of_line(index);
            } else if current == '/'
                && scanner.at(index + 1) == Some('*')
                && scanner.at(index + 2) != Some('!')
            {
                index = scanner.end_of_block_comment(index);
            } else if let Some((token, next)) = scanner.delimiter_command(index) {
                delimiter = token;
                index = next;
            } else if scanner.starts_with(index, &delimiter) {
                index += delimiter.len();
            } else {
                start = Some(index);
                significant_end = index;
            }
            continue;
        };

        if scanner.starts_with(index, &delimiter) {
            statements.push(scanner.statement(begun_at, significant_end));
            start = None;
            index += delimiter.len();
        } else if current == '\'' || current == '"' || current == '`' {
            index = scanner.end_of_quoted(index);
            significant_end = index;
        } else if scanner.is_line_comment(index) {
            index = scanner.end_of_line(index);
        } else if current == '/' && scanner.at(index + 1) == Some('*') {
            index = scanner.end_of_block_comment(index);
            significant_end = index;
        } else {
            index += 1;
            if !current.is_whitespace() {
                significant_end = index;
            }
        }
    }

    if let Some(begun_at) = start {
        statements.push(scanner.statement(begun_at, significant_end.min(scanner.chars.len())));
    }
    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<String> {
        split_statements(sql)
            .into_iter()
            .map(|statement| statement.text)
            .collect()
    }

    #[test]
    fn splits_on_semicolons_and_keeps_a_last_statement_without_one() {
        assert_eq!(
            texts("SELECT 1; SELECT 2;\nSELECT 3"),
            ["SELECT 1", "SELECT 2", "SELECT 3"]
        );
    }

    #[test]
    fn ignores_empty_statements_and_comment_only_chunks() {
        assert_eq!(
            texts(";; SELECT 1;;  ; -- nothing\n; /* x */ ; # y\n"),
            ["SELECT 1"]
        );
        assert!(split_statements("   \n -- just a comment").is_empty());
    }

    #[test]
    fn semicolons_in_strings_identifiers_and_comments_do_not_split() {
        let sql = "SELECT 'a;b', \"c;d\", `e;f` FROM t; /* ; */ SELECT 2 -- ;\n; SELECT 3 # ;";

        assert_eq!(
            texts(sql),
            [
                "SELECT 'a;b', \"c;d\", `e;f` FROM t",
                "SELECT 2",
                "SELECT 3"
            ]
        );
    }

    #[test]
    fn handles_escaped_and_doubled_quotes() {
        let sql = r"SELECT 'it\'s; fine', 'it''s; fine', `we``ird;`; SELECT 2";

        assert_eq!(
            texts(sql),
            [
                r"SELECT 'it\'s; fine', 'it''s; fine', `we``ird;`",
                "SELECT 2"
            ]
        );
    }

    #[test]
    fn double_dash_is_a_comment_only_before_whitespace() {
        assert_eq!(
            texts("SELECT 1--1; SELECT 2 -- note;\n"),
            ["SELECT 1--1", "SELECT 2"]
        );
    }

    #[test]
    fn executable_comments_are_statements_and_plain_leading_comments_are_dropped() {
        let sql = "/*!40101 SET NAMES utf8mb4 */;\n/* header */ SELECT 1;";

        assert_eq!(texts(sql), ["/*!40101 SET NAMES utf8mb4 */", "SELECT 1"]);
    }

    #[test]
    fn delimiter_command_lets_a_routine_body_contain_semicolons() {
        let sql = "DELIMITER //\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND//\nDELIMITER ;\nSELECT 3;";

        let statements = texts(sql);

        assert_eq!(statements.len(), 2);
        assert!(
            statements[0].starts_with("CREATE PROCEDURE p()") && statements[0].ends_with("END")
        );
        assert!(statements[0].contains("SELECT 1;\n  SELECT 2;"));
        assert_eq!(statements[1], "SELECT 3");
    }

    #[test]
    fn delimiter_can_be_several_characters_and_is_case_insensitive() {
        assert_eq!(
            texts("delimiter $$\nSELECT 1; SELECT 2$$\nDelimiter ;\nSELECT 3;"),
            ["SELECT 1; SELECT 2", "SELECT 3"]
        );
    }

    #[test]
    fn the_word_delimiter_inside_a_statement_is_not_a_command() {
        assert_eq!(
            texts("SELECT 1 AS delimiter x; SELECT delimiter FROM t"),
            ["SELECT 1 AS delimiter x", "SELECT delimiter FROM t"]
        );
    }

    #[test]
    fn positions_are_utf16_offsets_that_cover_the_statement_text() {
        let sql = "SELECT '😀'; SELECT 'é'";

        let statements = split_statements(sql);
        let units: Vec<u16> = sql.encode_utf16().collect();

        assert_eq!(statements.len(), 2);
        for statement in &statements {
            let slice = String::from_utf16(&units[statement.start..statement.end]).unwrap();
            assert_eq!(slice, statement.text);
        }
        assert_eq!(statements[0].end, 11); // ten characters, but the emoji is two UTF-16 units
    }

    #[test]
    fn unterminated_strings_and_comments_run_to_the_end_without_panicking() {
        assert_eq!(texts("SELECT 'abc; SELECT 2"), ["SELECT 'abc; SELECT 2"]);
        assert_eq!(
            texts("SELECT 1 /* open; SELECT 2"),
            ["SELECT 1 /* open; SELECT 2"]
        );
        assert_eq!(texts("SELECT 'a\\"), ["SELECT 'a\\"]);
    }
}
