//! Splits a script into statements as it is read in pieces, so a dump of any size can be imported
//! with bounded memory. It follows the same rules as `sql_split::split_statements` (strings, comments,
//! `DELIMITER` commands) and a test checks the two agree for every chunk size.

const MIN_LOOKAHEAD: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStatement {
    pub text: String,
    /// 1-based line of the statement's first character.
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Quoted(char),
    LineComment,
    BlockComment { in_statement: bool },
}

pub struct StatementStream {
    chars: Vec<char>,
    pos: usize,
    start: Option<usize>,
    /// Index just past the last character that belongs to the current statement.
    significant_end: usize,
    mode: Mode,
    delimiter: Vec<char>,
    /// Line number of `chars[0]`.
    first_line: usize,
}

impl Default for StatementStream {
    fn default() -> Self {
        Self {
            chars: Vec::new(),
            pos: 0,
            start: None,
            significant_end: 0,
            mode: Mode::Normal,
            delimiter: vec![';'],
            first_line: 1,
        }
    }
}

fn is_comment_dashes(chars: &[char], index: usize) -> bool {
    chars.get(index) == Some(&'-')
        && chars.get(index + 1) == Some(&'-')
        && chars
            .get(index + 2)
            .is_none_or(|c| c.is_whitespace() || c.is_control())
}

impl StatementStream {
    /// Feeds more text and returns the statements that are now complete.
    pub fn push(&mut self, text: &str) -> Vec<PendingStatement> {
        self.chars.extend(text.chars());
        let mut done = Vec::new();
        self.advance(false, &mut done);
        done
    }

    /// Ends the input: a last statement without a delimiter is returned too.
    pub fn finish(&mut self) -> Vec<PendingStatement> {
        let mut done = Vec::new();
        self.advance(true, &mut done);
        if matches!(
            self.mode,
            Mode::Quoted(_) | Mode::BlockComment { in_statement: true }
        ) {
            self.significant_end = self.chars.len();
        }
        if let Some(start) = self.start.take() {
            let end = self.significant_end.max(start).min(self.chars.len());
            done.push(self.statement(start, end));
        }
        self.chars.clear();
        self.pos = 0;
        done
    }

    /// Characters buffered but not yet part of a finished statement.
    #[cfg(test)]
    pub fn buffered(&self) -> usize {
        self.chars.len()
    }

    fn lookahead(&self) -> usize {
        MIN_LOOKAHEAD.max(self.delimiter.len() + 1)
    }

    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    fn starts_with(&self, index: usize, text: &[char]) -> bool {
        !text.is_empty() && self.chars.get(index..index + text.len()) == Some(text)
    }

    fn statement(&self, start: usize, end: usize) -> PendingStatement {
        let skipped = self.chars[..start].iter().filter(|c| **c == '\n').count();
        PendingStatement {
            text: self.chars[start..end].iter().collect(),
            line: self.first_line + skipped,
        }
    }

    fn delimiter_command(
        &self,
        index: usize,
        final_chunk: bool,
    ) -> Option<Option<(Vec<char>, usize)>> {
        const WORD: &str = "delimiter";
        let matches_word = WORD.chars().enumerate().all(|(offset, expected)| {
            self.at(index + offset)
                .is_some_and(|c| c.eq_ignore_ascii_case(&expected))
        });
        if !matches_word
            || !self
                .at(index + WORD.len())
                .is_some_and(|c| c == ' ' || c == '\t')
        {
            return Some(None);
        }
        let newline = self.chars[index..]
            .iter()
            .position(|c| *c == '\n')
            .map(|offset| index + offset);
        // The rest of the line has not arrived yet: wait for it.
        if newline.is_none() && !final_chunk {
            return None;
        }
        let line_end = newline.unwrap_or(self.chars.len());
        let token: String = self.chars[index + WORD.len()..line_end].iter().collect();
        let token: Vec<char> = token.trim().chars().collect();
        Some((!token.is_empty()).then_some((token, line_end)))
    }

    fn advance(&mut self, final_chunk: bool, done: &mut Vec<PendingStatement>) {
        loop {
            let available = self.chars.len() - self.pos;
            if available == 0 || (!final_chunk && available < self.lookahead()) {
                break;
            }
            let current = self.chars[self.pos];
            match self.mode {
                Mode::LineComment => {
                    if current == '\n' {
                        self.mode = Mode::Normal;
                    }
                    self.pos += 1;
                }
                Mode::BlockComment { in_statement } => {
                    if current == '*' && self.at(self.pos + 1) == Some('/') {
                        self.pos += 2;
                        self.mode = Mode::Normal;
                        if in_statement {
                            self.significant_end = self.pos;
                        }
                    } else {
                        self.pos += 1;
                    }
                }
                Mode::Quoted(quote) => {
                    if current == '\\' && quote != '`' {
                        self.pos = (self.pos + 2).min(self.chars.len());
                    } else if current == quote && self.at(self.pos + 1) == Some(quote) {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        if current == quote {
                            self.mode = Mode::Normal;
                        }
                    }
                    self.significant_end = self.pos;
                }
                Mode::Normal => {
                    if self.start.is_none() && !self.begin_statement(current, final_chunk) {
                        break;
                    }
                    if self.start.is_some() && self.mode == Mode::Normal {
                        self.continue_statement(current, done);
                    }
                }
            }
        }
        self.compact();
    }

    /// Between statements. Returns false when more input is needed before going on.
    fn begin_statement(&mut self, current: char, final_chunk: bool) -> bool {
        if current.is_whitespace() {
            self.pos += 1;
        } else if is_comment_dashes(&self.chars, self.pos) || current == '#' {
            self.mode = Mode::LineComment;
            self.pos += 1;
        } else if current == '/'
            && self.at(self.pos + 1) == Some('*')
            && self.at(self.pos + 2) != Some('!')
        {
            self.mode = Mode::BlockComment {
                in_statement: false,
            };
            self.pos += 2;
        } else {
            match self.delimiter_command(self.pos, final_chunk) {
                None => return false,
                Some(Some((token, next))) => {
                    self.delimiter = token;
                    self.pos = next;
                }
                Some(None) => {
                    let delimiter = self.delimiter.clone();
                    if self.starts_with(self.pos, &delimiter) {
                        self.pos += delimiter.len();
                    } else {
                        self.start = Some(self.pos);
                        self.significant_end = self.pos;
                    }
                }
            }
        }
        true
    }

    fn continue_statement(&mut self, current: char, done: &mut Vec<PendingStatement>) {
        let Some(start) = self.start else { return };
        let delimiter = self.delimiter.clone();
        if self.starts_with(self.pos, &delimiter) {
            done.push(self.statement(start, self.significant_end));
            self.start = None;
            self.pos += delimiter.len();
            // Dropping what was just returned keeps the buffer small; line numbers move along with it.
            self.compact();
        } else if current == '\'' || current == '"' || current == '`' {
            self.mode = Mode::Quoted(current);
            self.pos += 1;
            self.significant_end = self.pos;
        } else if is_comment_dashes(&self.chars, self.pos) || current == '#' {
            self.mode = Mode::LineComment;
            self.pos += 1;
        } else if current == '/' && self.at(self.pos + 1) == Some('*') {
            self.mode = Mode::BlockComment { in_statement: true };
            self.pos += 2;
        } else {
            self.pos += 1;
            if !current.is_whitespace() {
                self.significant_end = self.pos;
            }
        }
    }

    /// Forgets everything before the statement being read (or before the read position).
    fn compact(&mut self) {
        let keep_from = self.start.unwrap_or(self.pos).min(self.pos);
        if keep_from == 0 {
            return;
        }
        self.first_line += self.chars[..keep_from]
            .iter()
            .filter(|c| **c == '\n')
            .count();
        self.chars.drain(..keep_from);
        self.pos -= keep_from;
        self.significant_end = self.significant_end.saturating_sub(keep_from);
        if let Some(start) = &mut self.start {
            *start -= keep_from;
        }
    }
}

/// Turns bytes read in arbitrary pieces into text, holding back a character cut in two and
/// replacing bytes that are not valid UTF-8, so no piece boundary can split or lose a character.
#[derive(Default)]
pub struct Utf8Decoder {
    carry: Vec<u8>,
    started: bool,
}

const BYTE_ORDER_MARK: &str = "\u{feff}";

impl Utf8Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.carry.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = std::mem::take(&mut self.carry);
        loop {
            match std::str::from_utf8(&rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match error.error_len() {
                        None => {
                            self.carry = after.to_vec();
                            break;
                        }
                        Some(length) => {
                            text.push('\u{fffd}');
                            rest = after[length..].to_vec();
                        }
                    }
                }
            }
        }
        if !self.started && !text.is_empty() {
            self.started = true;
            if let Some(stripped) = text.strip_prefix(BYTE_ORDER_MARK) {
                return stripped.to_owned();
            }
        }
        text
    }

    /// Whatever is left at the end of the input was an incomplete character.
    pub fn finish(&mut self) -> String {
        if self.carry.is_empty() {
            String::new()
        } else {
            String::from_utf8_lossy(&std::mem::take(&mut self.carry)).into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_split::split_statements;

    fn stream_all(sql: &str, chunk_chars: usize) -> Vec<PendingStatement> {
        let mut stream = StatementStream::default();
        let chars: Vec<char> = sql.chars().collect();
        let mut statements = Vec::new();
        for piece in chars.chunks(chunk_chars) {
            statements.extend(stream.push(&piece.iter().collect::<String>()));
        }
        statements.extend(stream.finish());
        statements
    }

    const SCRIPTS: [&str; 12] = [
        "SELECT 1; SELECT 2;\nSELECT 3",
        ";; SELECT 1;;  ; -- nothing\n; /* x */ ; # y\n",
        "SELECT 'a;b', \"c;d\", `e;f` FROM t; /* ; */ SELECT 2 -- ;\n; SELECT 3 # ;",
        "SELECT 'it\\'s; fine', 'it''s; fine', `we``ird;`; SELECT 2",
        "SELECT 1--1; SELECT 2 -- note;\n",
        "/*!40101 SET NAMES utf8mb4 */;\n/* header */ SELECT 1;",
        "DELIMITER //\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND//\nDELIMITER ;\nSELECT 3;",
        "delimiter $$\nSELECT 1; SELECT 2$$\nDelimiter ;\nSELECT 3;",
        "SELECT 1 AS delimiter x; SELECT delimiter FROM t",
        "INSERT INTO t VALUES ('😀;é', 'x\\\\'), ('y');\nSELECT 'é';",
        "SELECT 'abc; SELECT 2",
        "SELECT 1 /* open; SELECT 2",
    ];

    #[test]
    fn streaming_agrees_with_splitting_the_whole_text_for_every_chunk_size() {
        for script in SCRIPTS {
            let expected: Vec<String> = split_statements(script)
                .into_iter()
                .map(|statement| statement.text)
                .collect();
            for chunk in [1, 2, 3, 5, 7, 16, 1000] {
                let got: Vec<String> = stream_all(script, chunk)
                    .into_iter()
                    .map(|statement| statement.text)
                    .collect();
                assert_eq!(got, expected, "script {script:?} in chunks of {chunk}");
            }
        }
    }

    #[test]
    fn statements_report_the_line_they_start_on() {
        let script = "-- header\nSELECT 1;\n\n/* c\nd */\nSELECT 2;\nSELECT\n  3;";

        for chunk in [1, 4, 1000] {
            let lines: Vec<usize> = stream_all(script, chunk)
                .into_iter()
                .map(|statement| statement.line)
                .collect();
            assert_eq!(lines, [2, 6, 7], "chunks of {chunk}");
        }
    }

    #[test]
    fn a_delimiter_split_across_pieces_is_still_found() {
        let mut stream = StatementStream::default();

        assert!(stream.push("DELIMITER ").is_empty());
        assert!(stream.push("$$\nSELECT 1; SEL").is_empty());
        let mut statements = stream.push("ECT 2$$\nSELECT 3;");
        // The last few characters wait for more input, so the end of the input is what releases them.
        statements.extend(stream.finish());

        // `;` is no longer the delimiter, so the last statement keeps it.
        assert_eq!(
            statements
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["SELECT 1; SELECT 2", "SELECT 3;"]
        );
    }

    #[test]
    fn memory_stays_small_while_reading_many_statements() {
        let mut stream = StatementStream::default();
        let mut total = 0;

        for round in 0..2000 {
            total += stream
                .push(&format!("INSERT INTO t VALUES ({round}, 'some text');\n"))
                .len();
            assert!(
                stream.buffered() < 200,
                "buffered {} characters",
                stream.buffered()
            );
        }

        total += stream.finish().len();

        assert_eq!(total, 2000);
    }

    #[test]
    fn the_decoder_never_splits_or_loses_a_character() {
        let text = "héllo 😀 wörld";
        let bytes = text.as_bytes();

        for cut in 1..bytes.len() {
            let mut decoder = Utf8Decoder::default();
            let joined =
                decoder.push(&bytes[..cut]) + &decoder.push(&bytes[cut..]) + &decoder.finish();
            assert_eq!(joined, text, "cut at {cut}");
        }
    }

    #[test]
    fn invalid_bytes_become_replacement_characters_and_a_bom_is_dropped() {
        let mut decoder = Utf8Decoder::default();

        let text = decoder.push(b"\xef\xbb\xbfab\xffcd");

        assert_eq!(text, "ab\u{fffd}cd");
        assert_eq!(Utf8Decoder::default().push("\u{feff}x".as_bytes()), "x");
        let mut cut_off = Utf8Decoder::default();
        cut_off.push(&"é".as_bytes()[..1]);
        assert_eq!(cut_off.finish(), "\u{fffd}");
    }
}
