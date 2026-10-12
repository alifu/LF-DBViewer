//! Small helpers for building SQL text safely.

/// Quotes an identifier with backticks, doubling any backtick inside it.
pub fn quote_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// `` `database`.`object` ``
pub fn qualified(database: &str, object: &str) -> String {
    format!(
        "{}.{}",
        quote_identifier(database),
        quote_identifier(object)
    )
}

/// A single-quoted string literal: quotes are doubled and backslashes escaped.
pub fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

/// Letters, digits and underscores only: safe to place unquoted (engine, collation, row format names).
pub fn is_plain_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// True when every quote and parenthesis opened in `text` is closed again, so the text
/// cannot swallow what follows it in a statement.
pub fn is_balanced(text: &str) -> bool {
    let (mut depth, mut quote) = (0i32, None);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), '\\') if open != '`' => {
                chars.next();
            }
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            (None, _) => {}
        }
    }
    depth == 0 && quote.is_none()
}

pub fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_escapes_backticks() {
        assert_eq!(quote_identifier("orders"), "`orders`");
        assert_eq!(quote_identifier("we`ird"), "`we``ird`");
        assert_eq!(qualified("shop", "order items"), "`shop`.`order items`");
    }

    #[test]
    fn string_literals_escape_quotes_and_backslashes() {
        assert_eq!(quote_string("it's"), "'it''s'");
        assert_eq!(quote_string(r"a\b"), r"'a\\b'");
        assert_eq!(quote_string(""), "''");
    }

    #[test]
    fn plain_words_are_letters_digits_and_underscores() {
        assert!(is_plain_word("utf8mb4_0900_ai_ci"));
        assert!(!is_plain_word("a b"));
        assert!(!is_plain_word("x;drop"));
        assert!(!is_plain_word(""));
    }

    #[test]
    fn balance_check_understands_quotes_and_parentheses() {
        assert!(is_balanced("varchar(255)"));
        assert!(is_balanced("enum('a)','b(')"));
        assert!(is_balanced(r"enum('it\'s')"));
        assert!(!is_balanced("varchar(255"));
        assert!(!is_balanced("a) b ("));
        assert!(!is_balanced("enum('a"));
        assert!(!is_balanced("x'; DROP TABLE t; --"));
    }

    #[test]
    fn placeholders_are_comma_separated() {
        assert_eq!(placeholders(3), "?, ?, ?");
        assert_eq!(placeholders(0), "");
    }
}
