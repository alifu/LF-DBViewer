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
    fn placeholders_are_comma_separated() {
        assert_eq!(placeholders(3), "?, ?, ?");
        assert_eq!(placeholders(0), "");
    }
}
