//! How one value is written in an `INSERT`. The aim is that loading the file gives back exactly what was read:
//! binary data as hex, text escaped, decimals as typed, floats in a form that reads back to the same number.

use mysql_async::consts::ColumnType;
use mysql_async::{Column, Value};

use crate::query::cell::{Cell, ColumnShape, TEXT, cell_from_value};

/// What kind of number a column holds. The server's text protocol sends every value as text, so this is how a
/// number is told from a string: numbers are written without quotes, and in JSON as numbers (decimals excepted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberClass {
    NotANumber,
    Integer,
    Float,
    /// Exact decimals: kept as text everywhere, so no digit is lost.
    Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpColumn {
    pub shape: ColumnShape,
    pub number: NumberClass,
}

impl DumpColumn {
    pub fn of(column: &Column) -> Self {
        let number = match column.column_type() {
            ColumnType::MYSQL_TYPE_TINY
            | ColumnType::MYSQL_TYPE_SHORT
            | ColumnType::MYSQL_TYPE_LONG
            | ColumnType::MYSQL_TYPE_LONGLONG
            | ColumnType::MYSQL_TYPE_INT24
            | ColumnType::MYSQL_TYPE_YEAR => NumberClass::Integer,
            ColumnType::MYSQL_TYPE_FLOAT | ColumnType::MYSQL_TYPE_DOUBLE => NumberClass::Float,
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => {
                NumberClass::Decimal
            }
            _ => NumberClass::NotANumber,
        };
        Self {
            shape: ColumnShape::of(column),
            number,
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
        Value::Bytes(bytes) if column.number != NumberClass::NotANumber && looks_numeric(bytes) => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        Value::Bytes(bytes) => format!("'{}'", escape_string(&String::from_utf8_lossy(bytes))),
    }
}

/// A value as plain text for CSV, with `None` for NULL. Binary values are written as `0x…` hex.
pub fn plain_text(value: &Value, column: DumpColumn) -> Option<String> {
    match value {
        Value::NULL => None,
        Value::Bytes(bytes) if column.shape.is_binary => Some(if bytes.is_empty() {
            String::new()
        } else {
            hex_literal(bytes)
        }),
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Value::Int(number) => Some(number.to_string()),
        Value::UInt(number) => Some(number.to_string()),
        Value::Float(number) => Some(number.to_string()),
        Value::Double(number) => Some(number.to_string()),
        Value::Date(..) | Value::Time(..) => match cell_from_value(
            value.clone(),
            if column.shape.is_date_only {
                column.shape
            } else {
                TEXT
            },
        ) {
            Cell::Text(text) => Some(text),
            _ => None,
        },
    }
}

/// A value as a JSON token: numbers stay numbers, everything else is a string, NULL is `null`.
/// Decimals are strings on purpose, because a JSON number would be read back as a float.
pub fn json_value(value: &Value, column: DumpColumn) -> serde_json::Value {
    match value {
        Value::NULL => serde_json::Value::Null,
        Value::Int(number) => (*number).into(),
        Value::UInt(number) => (*number).into(),
        Value::Float(number) => serde_json::Number::from_f64(f64::from(*number))
            .map_or(serde_json::Value::Null, Into::into),
        Value::Double(number) => {
            serde_json::Number::from_f64(*number).map_or(serde_json::Value::Null, Into::into)
        }
        Value::Bytes(bytes) if column.number == NumberClass::Integer => {
            let text = String::from_utf8_lossy(bytes);
            text.parse::<i64>()
                .map(Into::into)
                .or_else(|_| text.parse::<u64>().map(Into::into))
                .unwrap_or_else(|_| text.into_owned().into())
        }
        Value::Bytes(bytes) if column.number == NumberClass::Float => {
            let text = String::from_utf8_lossy(bytes);
            text.parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| text.into_owned().into(), Into::into)
        }
        other => plain_text(other, column).map_or(serde_json::Value::Null, Into::into),
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
        number: NumberClass::NotANumber,
    };
    const BINARY_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: true,
            is_date_only: false,
        },
        number: NumberClass::NotANumber,
    };
    const DATE_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: true,
        },
        number: NumberClass::NotANumber,
    };
    const DECIMAL_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: false,
        },
        number: NumberClass::Decimal,
    };
    const INTEGER_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: false,
        },
        number: NumberClass::Integer,
    };
    const FLOAT_COLUMN: DumpColumn = DumpColumn {
        shape: ColumnShape {
            is_binary: false,
            is_date_only: false,
        },
        number: NumberClass::Float,
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
    fn plain_text_has_no_quoting_and_json_keeps_numbers_as_numbers() {
        assert_eq!(
            plain_text(&text("it's, \"x\""), TEXT_COLUMN).as_deref(),
            Some("it's, \"x\"")
        );
        assert_eq!(plain_text(&Value::NULL, TEXT_COLUMN), None);
        assert_eq!(
            plain_text(&Value::Bytes(vec![0xca, 0xfe]), BINARY_COLUMN).as_deref(),
            Some("0xcafe")
        );
        assert_eq!(
            plain_text(&Value::Date(2026, 1, 31, 0, 0, 0, 0), DATE_COLUMN).as_deref(),
            Some("2026-01-31")
        );
        assert_eq!(
            json_value(&Value::UInt(u64::MAX), TEXT_COLUMN),
            serde_json::json!(18446744073709551615u64)
        );
        assert_eq!(
            json_value(&text("12.50"), DECIMAL_COLUMN),
            serde_json::json!("12.50")
        );
        assert_eq!(
            json_value(&Value::Double(f64::NAN), TEXT_COLUMN),
            serde_json::Value::Null
        );
        assert_eq!(
            json_value(&Value::NULL, TEXT_COLUMN),
            serde_json::Value::Null
        );
        assert_eq!(
            json_value(&Value::Double(0.5), TEXT_COLUMN),
            serde_json::json!(0.5)
        );
    }

    #[test]
    fn numbers_that_arrive_as_text_are_written_as_numbers_and_strings_stay_strings() {
        assert_eq!(literal(&text("42"), INTEGER_COLUMN), "42");
        assert_eq!(literal(&text("-7"), INTEGER_COLUMN), "-7");
        assert_eq!(literal(&text("0.1"), FLOAT_COLUMN), "0.1");
        assert_eq!(literal(&text("1e30"), FLOAT_COLUMN), "1e30");
        assert_eq!(literal(&text("42"), TEXT_COLUMN), "'42'");
        assert_eq!(literal(&text("4; DROP"), INTEGER_COLUMN), "'4; DROP'");
        assert_eq!(
            json_value(&text("42"), INTEGER_COLUMN),
            serde_json::json!(42)
        );
        assert_eq!(
            json_value(&text("18446744073709551615"), INTEGER_COLUMN),
            serde_json::json!(18446744073709551615u64)
        );
        assert_eq!(
            json_value(&text("0.25"), FLOAT_COLUMN),
            serde_json::json!(0.25)
        );
        assert_eq!(
            json_value(&text("42"), TEXT_COLUMN),
            serde_json::json!("42")
        );
        assert_eq!(
            json_value(&text("12.50"), DECIMAL_COLUMN),
            serde_json::json!("12.50")
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
