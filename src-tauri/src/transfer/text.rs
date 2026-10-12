//! The fixed text of a dump file and the helpers that shape statements in it.

use crate::sql::quote_identifier;

pub const DEFAULT_STATEMENT_BYTES: usize = 1_000_000;

/// The session settings a file relies on, saved first so the footer can put them back. The same set
/// `mysqldump` writes: the connection character set, UTC for timestamps, and no checks that would reject rows
/// loaded out of order or with zero dates.
pub fn header(server_version: &str, generated_at: &str, database: &str) -> String {
    format!(
        "-- LF-DBViewer SQL dump\n-- Server version: {server_version}\n-- Database: {database}\n-- Generated: {generated_at}\n\n\
         /*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;\n\
         /*!40101 SET @OLD_CHARACTER_SET_RESULTS=@@CHARACTER_SET_RESULTS */;\n\
         /*!40101 SET @OLD_COLLATION_CONNECTION=@@COLLATION_CONNECTION */;\n\
         /*!40101 SET NAMES utf8mb4 */;\n\
         /*!40103 SET @OLD_TIME_ZONE=@@TIME_ZONE */;\n\
         /*!40103 SET TIME_ZONE='+00:00' */;\n\
         /*!40014 SET @OLD_UNIQUE_CHECKS=@@UNIQUE_CHECKS, UNIQUE_CHECKS=0 */;\n\
         /*!40014 SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS=0 */;\n\
         /*!40101 SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO' */;\n\
         /*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;\n\n"
    )
}

pub const FOOTER: &str = "\n/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;\n\
/*!40014 SET FOREIGN_KEY_CHECKS=@OLD_FOREIGN_KEY_CHECKS */;\n\
/*!40014 SET UNIQUE_CHECKS=@OLD_UNIQUE_CHECKS */;\n\
/*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */;\n\
/*!40101 SET CHARACTER_SET_RESULTS=@OLD_CHARACTER_SET_RESULTS */;\n\
/*!40101 SET COLLATION_CONNECTION=@OLD_COLLATION_CONNECTION */;\n\
/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */;\n\
/*!40111 SET SQL_NOTES=@OLD_SQL_NOTES */;\n\
-- Dump completed\n";

pub fn section(title: &str) -> String {
    format!("\n--\n-- {title}\n--\n\n")
}

/// Removes `DEFINER=user@host` from the start of a `CREATE` statement, so the object belongs to whoever
/// imports the file instead of failing when that account does not exist (or lacks the privilege to impersonate it).
/// Only the statement's head is searched, so text inside a routine body is never touched.
pub fn strip_definer(sql: &str) -> String {
    const HEAD_CHARS: usize = 512;
    let head_end = sql
        .char_indices()
        .nth(HEAD_CHARS)
        .map_or(sql.len(), |(index, _)| index);
    let head = &sql[..head_end];
    let lower = head.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find("definer") {
        let start = search_from + found;
        let after_word = start + "definer".len();
        let rest = head[after_word..].trim_start();
        let Some(account) = rest.strip_prefix('=') else {
            search_from = after_word;
            continue;
        };
        let account = account.trim_start();
        let length = account_length(account);
        if length == 0 {
            search_from = after_word;
            continue;
        }
        let consumed = head.len() - account.len() + length;
        let mut end = consumed;
        while sql[end..].starts_with(' ') {
            end += 1;
        }
        return format!("{}{}", &sql[..start], &sql[end..]);
    }
    sql.to_owned()
}

/// Length of `` `user`@`host` ``, `user@host` or `CURRENT_USER[()]` at the start of `text`.
fn account_length(text: &str) -> usize {
    fn part(text: &str) -> usize {
        if let Some(rest) = text.strip_prefix('`') {
            let mut index = 0;
            let bytes = rest.as_bytes();
            while index < bytes.len() {
                if bytes[index] == b'`' {
                    if bytes.get(index + 1) == Some(&b'`') {
                        index += 2;
                        continue;
                    }
                    return index + 2;
                }
                index += 1;
            }
            return 0;
        }
        text.find(|c: char| c == '@' || c.is_whitespace())
            .unwrap_or(text.len())
    }
    if text.to_ascii_lowercase().starts_with("current_user") {
        let base = "current_user".len();
        return if text[base..].starts_with("()") {
            base + 2
        } else {
            base
        };
    }
    let user = part(text);
    if user == 0 || !text[user..].starts_with('@') {
        return 0;
    }
    let host = part(&text[user + 1..]);
    if host == 0 { 0 } else { user + 1 + host }
}

/// Builds `INSERT` statements that stay under a size limit, so a statement never exceeds the importing server's
/// `max_allowed_packet`, while still carrying many rows each (far faster to load than one row per statement).
pub struct InsertBatcher {
    prefix: String,
    max_bytes: usize,
    body: String,
    rows: usize,
}

impl InsertBatcher {
    pub fn new(table: &str, columns: &[String], max_bytes: usize) -> Self {
        let names: Vec<String> = columns.iter().map(|name| quote_identifier(name)).collect();
        Self {
            prefix: format!(
                "INSERT INTO {} ({}) VALUES ",
                quote_identifier(table),
                names.join(",")
            ),
            max_bytes,
            body: String::new(),
            rows: 0,
        }
    }

    /// Adds a row; returns a finished statement when adding it would make the current one too big.
    pub fn push(&mut self, row: &str) -> Option<String> {
        let full =
            self.rows > 0 && self.prefix.len() + self.body.len() + row.len() + 2 > self.max_bytes;
        let finished = full.then(|| self.take());
        if self.rows > 0 {
            self.body.push(',');
        }
        self.body.push_str(row);
        self.rows += 1;
        finished
    }

    pub fn finish(&mut self) -> Option<String> {
        (self.rows > 0).then(|| self.take())
    }

    fn take(&mut self) -> String {
        let statement = format!("{}{};\n", self.prefix, self.body);
        self.body.clear();
        self.rows = 0;
        statement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definers_are_removed_from_the_head_of_every_kind_of_statement() {
        let cases = [
            (
                "CREATE DEFINER=`root`@`localhost` PROCEDURE `p`() SELECT 1",
                "CREATE PROCEDURE `p`() SELECT 1",
            ),
            (
                "CREATE ALGORITHM=UNDEFINED DEFINER=`app`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1",
                "CREATE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v` AS select 1",
            ),
            (
                "CREATE DEFINER = root@localhost TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1",
                "CREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1",
            ),
            (
                "CREATE DEFINER=CURRENT_USER() EVENT e ON SCHEDULE EVERY 1 DAY DO SELECT 1",
                "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO SELECT 1",
            ),
            (
                "CREATE DEFINER=`we``ird`@`h@st` FUNCTION f() RETURNS INT RETURN 1",
                "CREATE FUNCTION f() RETURNS INT RETURN 1",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(strip_definer(input), expected, "{input}");
        }
    }

    #[test]
    fn statements_without_a_definer_and_text_deep_in_a_body_are_left_alone() {
        let body = format!(
            "CREATE PROCEDURE p() BEGIN {} SELECT 'DEFINER=`x`@`y`'; END",
            " ".repeat(600)
        );

        assert_eq!(
            strip_definer("CREATE PROCEDURE p() SELECT 1"),
            "CREATE PROCEDURE p() SELECT 1"
        );
        assert_eq!(
            strip_definer("CREATE VIEW v AS SELECT 1 AS definer"),
            "CREATE VIEW v AS SELECT 1 AS definer"
        );
        assert_eq!(strip_definer(&body), body);
    }

    #[test]
    fn a_batch_stays_under_the_limit_and_keeps_every_row() {
        let columns = vec!["id".to_owned(), "name".to_owned()];
        let mut batcher = InsertBatcher::new("t", &columns, 100);
        let mut statements = Vec::new();

        for id in 0..10 {
            statements.extend(batcher.push(&format!("({id},'name number {id}')")));
        }
        statements.extend(batcher.finish());

        assert!(statements.len() > 1);
        assert!(
            statements
                .iter()
                .all(|statement| statement.len() <= 100 + 2)
        );
        assert!(statements.iter().all(|statement| {
            statement.starts_with("INSERT INTO `t` (`id`,`name`) VALUES (")
                && statement.ends_with(");\n")
        }));
        let rows: usize = statements
            .iter()
            .map(|statement| statement.matches("'name number").count())
            .sum();
        assert_eq!(rows, 10);
    }

    #[test]
    fn a_row_bigger_than_the_limit_still_goes_out_alone() {
        let mut batcher = InsertBatcher::new("t", &["a".to_owned()], 20);

        assert!(
            batcher
                .push("('this row is longer than the limit')")
                .is_none()
        );
        let first = batcher.push("(1)").unwrap();
        let last = batcher.finish().unwrap();

        assert!(first.contains("this row is longer"));
        assert_eq!(last, "INSERT INTO `t` (`a`) VALUES (1);\n");
        assert!(batcher.finish().is_none());
    }

    #[test]
    fn the_header_and_footer_pair_up_the_settings_they_change() {
        let text = header("9.3.0", "2026-10-12 10:00:00 UTC", "shop");

        for setting in [
            "CHARACTER_SET_CLIENT",
            "TIME_ZONE",
            "UNIQUE_CHECKS",
            "FOREIGN_KEY_CHECKS",
            "SQL_MODE",
            "SQL_NOTES",
        ] {
            assert!(
                text.contains(&format!("@OLD_{setting}="))
                    || text.contains(&format!("@OLD_{setting}=@@")),
                "{setting}"
            );
            assert!(FOOTER.contains(&format!("@OLD_{setting}")), "{setting}");
        }
        assert!(text.contains("TIME_ZONE='+00:00'") && text.contains("SET NAMES utf8mb4"));
    }
}
