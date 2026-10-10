//! Converts driver values into what the grid displays. Everything is sent as text so
//! big integers and decimals keep their exact digits; large and binary values are cut
//! down to a preview so one huge cell cannot bloat a page of rows.

use mysql_async::consts::ColumnType;
use mysql_async::{Column, Value};
use serde::Serialize;

pub const MAX_TEXT_CHARS: usize = 2_000;
pub const BINARY_PREVIEW_BYTES: usize = 32;
const BINARY_CHARSET: u16 = 63;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecialKind {
    /// Text longer than the limit; `preview` holds the beginning.
    Truncated,
    /// Binary data; `preview` holds the first bytes as hex.
    Binary,
}

/// Serializes as `null`, a string, or `{ kind, preview, size }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Null,
    Text(String),
    Special {
        kind: SpecialKind,
        preview: String,
        size: usize,
    },
}

/// What the conversion needs to know about a result column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnShape {
    pub is_binary: bool,
    pub is_date_only: bool,
}

impl ColumnShape {
    pub fn of(column: &Column) -> Self {
        let column_type = column.column_type();
        let holds_bytes = matches!(
            column_type,
            ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
                | ColumnType::MYSQL_TYPE_VAR_STRING
                | ColumnType::MYSQL_TYPE_STRING
                | ColumnType::MYSQL_TYPE_VARCHAR
        );
        Self {
            is_binary: matches!(
                column_type,
                ColumnType::MYSQL_TYPE_GEOMETRY | ColumnType::MYSQL_TYPE_BIT
            ) || (holds_bytes && column.character_set() == BINARY_CHARSET),
            is_date_only: column_type == ColumnType::MYSQL_TYPE_DATE,
        }
    }
}

fn fraction(micros: u32) -> String {
    if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn text_cell(bytes: Vec<u8>) -> Cell {
    let text = String::from_utf8_lossy(&bytes);
    match text.char_indices().nth(MAX_TEXT_CHARS) {
        None => Cell::Text(text.into_owned()),
        Some((cut, _)) => Cell::Special {
            kind: SpecialKind::Truncated,
            preview: text[..cut].to_owned(),
            size: bytes.len(),
        },
    }
}

fn binary_cell(bytes: &[u8]) -> Cell {
    Cell::Special {
        kind: SpecialKind::Binary,
        preview: hex(&bytes[..bytes.len().min(BINARY_PREVIEW_BYTES)]),
        size: bytes.len(),
    }
}

pub fn cell_from_value(value: Value, shape: ColumnShape) -> Cell {
    match value {
        Value::NULL => Cell::Null,
        Value::Bytes(bytes) if shape.is_binary => binary_cell(&bytes),
        Value::Bytes(bytes) => text_cell(bytes),
        Value::Int(number) => Cell::Text(number.to_string()),
        Value::UInt(number) => Cell::Text(number.to_string()),
        Value::Float(number) => Cell::Text(number.to_string()),
        Value::Double(number) => Cell::Text(number.to_string()),
        Value::Date(year, month, day, ..) if shape.is_date_only => {
            Cell::Text(format!("{year:04}-{month:02}-{day:02}"))
        }
        Value::Date(year, month, day, hour, minute, second, micros) => Cell::Text(format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}{}",
            fraction(micros)
        )),
        Value::Time(negative, days, hours, minutes, seconds, micros) => Cell::Text(format!(
            "{}{:02}:{minutes:02}:{seconds:02}{}",
            if negative { "-" } else { "" },
            u32::from(hours) + days * 24,
            fraction(micros)
        )),
    }
}

/// Plain text of a value, for metadata queries where every column is short text or a number.
pub fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::NULL => None,
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        other => match cell_from_value(other.clone(), TEXT) {
            Cell::Text(text) => Some(text),
            _ => None,
        },
    }
}

pub const TEXT: ColumnShape = ColumnShape {
    is_binary: false,
    is_date_only: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    const BINARY: ColumnShape = ColumnShape {
        is_binary: true,
        is_date_only: false,
    };
    const DATE: ColumnShape = ColumnShape {
        is_binary: false,
        is_date_only: true,
    };

    #[test]
    fn numbers_keep_every_digit_as_text() {
        assert_eq!(
            cell_from_value(Value::UInt(u64::MAX), TEXT),
            Cell::Text("18446744073709551615".into())
        );
        assert_eq!(
            cell_from_value(Value::Int(-7), TEXT),
            Cell::Text("-7".into())
        );
        assert_eq!(
            cell_from_value(
                Value::Bytes(b"12345678901234567890.123456789".to_vec()),
                TEXT
            ),
            Cell::Text("12345678901234567890.123456789".into())
        );
    }

    #[test]
    fn dates_and_times_are_formatted_like_mysql() {
        assert_eq!(
            cell_from_value(Value::Date(2026, 3, 9, 0, 0, 0, 0), DATE),
            Cell::Text("2026-03-09".into())
        );
        assert_eq!(
            cell_from_value(Value::Date(2026, 3, 9, 7, 5, 1, 0), TEXT),
            Cell::Text("2026-03-09 07:05:01".into())
        );
        assert_eq!(
            cell_from_value(Value::Date(2026, 3, 9, 7, 5, 1, 250), TEXT),
            Cell::Text("2026-03-09 07:05:01.000250".into())
        );
        assert_eq!(
            cell_from_value(Value::Time(true, 1, 2, 3, 4, 0), TEXT),
            Cell::Text("-26:03:04".into())
        );
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_TEXT_CHARS + 10);

        let cell = cell_from_value(Value::Bytes(long.clone().into_bytes()), TEXT);

        let Cell::Special {
            kind,
            preview,
            size,
        } = cell
        else {
            panic!("expected a truncated cell")
        };
        assert_eq!(kind, SpecialKind::Truncated);
        assert_eq!(preview.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(size, long.len());
    }

    #[test]
    fn binary_data_becomes_a_hex_preview() {
        let cell = cell_from_value(Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]), BINARY);

        assert_eq!(
            cell,
            Cell::Special {
                kind: SpecialKind::Binary,
                preview: "deadbeef".into(),
                size: 4
            }
        );
    }

    #[test]
    fn cells_serialize_as_null_string_or_object() {
        let cells = vec![Cell::Null, Cell::Text("a".into()), binary_cell(&[1])];

        let json = serde_json::to_string(&cells).unwrap();

        assert_eq!(
            json,
            r#"[null,"a",{"kind":"binary","preview":"01","size":1}]"#
        );
    }

    #[test]
    fn value_text_reads_numbers_and_skips_null() {
        assert_eq!(value_text(&Value::UInt(42)).as_deref(), Some("42"));
        assert_eq!(value_text(&Value::NULL), None);
    }
}
