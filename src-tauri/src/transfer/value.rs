//! How one value is written in an `INSERT`. The aim is that loading the file gives back exactly what was read:
//! binary data as hex, text escaped, decimals as typed, floats in a form that reads back to the same number.

use mysql_async::consts::ColumnType;
use mysql_async::{Column, Value};

use crate::query::cell::{Cell, ColumnShape, TEXT, cell_from_value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpColumn {
    pub shape: ColumnShape,
    /// DECIMAL values arrive as text but are numbers, so they are written without quotes.
    pub numeric_text: bool,
}

impl DumpColumn {
    pub fn of(column: &Column) -> Self {
        let numeric_text = matches!(
            column.column_type(),
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL
        );
        Self {
            shape: ColumnShape::of(column),
            numeric_text,
        }
    }
}

/// Escapes what could end or corrupt a quoted string. Doing it by hand (rather than relying on a mode)
/// keeps the file valid whatever the importing session's settings are, apart from NO_BACKSLASH_ESCAPES.
pub fn escape_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\'' => escaped.push_str("\\'"),
            '\0' => escaped.push_str("\\0"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\u{1a}' => escaped.push_str("\\Z"),
            other => escaped.push(other),
        }
    }
    escaped
}

pub fn hex_literal(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "''".to_owned();
    }
    let mut literal = String::with_capacity(bytes.len() * 2 + 2);
    literal.push_str("0x");
    for byte in bytes {
        literal.push_str(&format!("{byte:02x}"));
    }
    literal
}

fn looks_numeric(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E'))
}

/// Floats in scientific form: it is the shortest text that reads back to the same number, and it never
/// turns a huge exponent into hundreds of digits.
fn float_literal(value: f64) -> String {
    if value.is_finite() {
        format!("{value:e}")
    } else {
        "NULL".to_owned()
    }
}

pub fn literal(value: &Value, column: DumpColumn) -> String {
    match value {
        Value::NULL => "NULL".to_owned(),
        Value::Int(number) => number.to_string(),
        Value::UInt(number) => number.to_string(),
        Value::Float(number) => {
            // The shortest text for a 32-bit float, not for the widened 64-bit value.
            if number.is_finite() {
                format!("{number:e}")
            } else {
                "NULL".to_owned()
            }
        }
        Value::Double(number) => float_literal(*number),
        Value::Date(..) | Value::Time(..) => match cell_from_value(
            value.clone(),
            if column.shape.is_date_only {
                column.shape
            } else {
                TEXT
            },
        ) {
            Cell::Text(text) => format!("'{text}'"),
            _ => "NULL".to_owned(),
        },
        Value::Bytes(bytes) if column.shape.is_binary => hex_literal(bytes),
        Value::Bytes(bytes) if column.numeric_text && looks_numeric(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        Value::Bytes(bytes) => format!("'{}'", escape_string(&String::from_utf8_lossy(bytes))),
    }
}

/// `(v1,v2,…)`
pub fn row_literal(values: &[Value], columns: &[DumpColumn]) -> String {
    let parts: Vec<String> = values
        .iter()
        .zip(columns)
        .map(|(value, column)| literal(value, *column))
        .collect();
    format!("({})", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: false,
        },
        numeric_text: false,
    };
    const BINARY_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: true,
            is_date_only: false,
        },
        numeric_text: false,
    };
    const DATE_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: true,
        },
        numeric_text: false,
    };
    const DECIMAL_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: false,
        },
        numeric_text: true,
    };

    fn text(value: &str) -> Value {
        Value::Bytes(value.as_bytes().to_vec())
    }

    #[test]
    fn strings_are_escaped_so_they_cannot_end_the_literal() {
        assert_eq!(literal(&text("it's"), TEXT_COLUMN), r"'it\'s'");
        assert_eq!(literal(&text("a\\b"), TEXT_COLUMN), r"'a\\b'");
        assert_eq!(
            literal(&text("line1\nline2\r\0\u{1a}"), TEXT_COLUMN),
            r"'line1\nline2\r\0\Z'"
        );
        assert_eq!(literal(&text("😀 é"), TEXT_COLUMN), "'😀 é'");
        assert_eq!(
            literal(&text("'); DROP TABLE t; --"), TEXT_COLUMN),
            r"'\'); DROP TABLE t; --'"
        );
    }

    #[test]
    fn binary_values_are_hex_and_empty_ones_are_an_empty_string() {
        assert_eq!(
            literal(&Value::Bytes(vec![0xde, 0xad, 0x00, 0xff]), BINARY_COLUMN),
            "0xdead00ff"
        );
        assert_eq!(literal(&Value::Bytes(vec![]), BINARY_COLUMN), "''");
    }

    #[test]
    fn integers_keep_every_digit_and_null_is_null() {
        assert_eq!(
            literal(&Value::UInt(u64::MAX), TEXT_COLUMN),
            "18446744073709551615"
        );
        assert_eq!(
            literal(&Value::Int(i64::MIN), TEXT_COLUMN),
            "-9223372036854775808"
        );
        assert_eq!(literal(&Value::NULL, TEXT_COLUMN), "NULL");
    }

    #[test]
    fn decimals_are_unquoted_numbers_and_other_text_that_looks_numeric_stays_quoted() {
        assert_eq!(
            literal(&text("12345678901234567890.0123456789"), DECIMAL_COLUMN),
            "12345678901234567890.0123456789"
        );
        assert_eq!(literal(&text("-0.50"), DECIMAL_COLUMN), "-0.50");
        assert_eq!(literal(&text("123"), TEXT_COLUMN), "'123'");
        assert_eq!(literal(&text("1; DROP"), DECIMAL_COLUMN), "'1; DROP'");
    }

    #[test]
    fn floats_are_written_so_they_read_back_exactly() {
        for value in [
            0.1_f64,
            1.0 / 3.0,
            1e300,
            -2.5e-300,
            0.0,
            123_456_789.123_456_79,
            f64::MAX,
            f64::MIN_POSITIVE,
        ] {
            let written = literal(&Value::Double(value), TEXT_COLUMN);
            assert_eq!(written.parse::<f64>().unwrap(), value, "{written}");
        }
        for value in [0.1_f32, 2.5, 16_777_217.0, 1e-30] {
            let written = literal(&Value::Float(value), TEXT_COLUMN);
            assert_eq!(written.parse::<f32>().unwrap(), value, "{written}");
        }
        assert_eq!(literal(&Value::Double(1e300), TEXT_COLUMN), "1e300");
        assert_eq!(literal(&Value::Double(f64::NAN), TEXT_COLUMN), "NULL");
    }

    #[test]
    fn dates_and_times_are_quoted_in_mysql_format() {
        assert_eq!(
            literal(&Value::Date(2026, 1, 31, 0, 0, 0, 0), DATE_COLUMN),
            "'2026-01-31'"
        );
        assert_eq!(
            literal(&Value::Date(2026, 1, 31, 13, 5, 9, 250), TEXT_COLUMN),
            "'2026-01-31 13:05:09.000250'"
        );
        assert_eq!(
            literal(&Value::Date(0, 0, 0, 0, 0, 0, 0), TEXT_COLUMN),
            "'0000-00-00 00:00:00'"
        );
        assert_eq!(
            literal(&Value::Time(true, 1, 2, 3, 4, 0), TEXT_COLUMN),
            "'-26:03:04'"
        );
    }

    #[test]
    fn a_row_is_a_parenthesised_list() {
        let row = row_literal(
            &[Value::Int(1), text("a"), Value::NULL],
            &[TEXT_COLUMN, TEXT_COLUMN, TEXT_COLUMN],
        );

        assert_eq!(row, "(1,'a',NULL)");
    }
}
