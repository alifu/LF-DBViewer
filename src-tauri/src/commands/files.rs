use std::path::{Path, PathBuf};

use tauri::ipc::{InvokeBody, Request};

use crate::error::{AppError, AppResult};

/// Only these kinds of file can be written from the interface: pictures made by the diagram export.
const ALLOWED_EXTENSIONS: [&str; 2] = ["svg", "png"];
const PATH_HEADER: &str = "path";

/// Decodes `%XX` escapes (UTF-8) as used in a URL; a header cannot carry arbitrary text otherwise.
pub fn percent_decode(text: &str) -> AppResult<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text
                .get(index + 1..index + 3)
                .ok_or_else(|| AppError::Validation("Bad path encoding".into()))?;
            decoded.push(
                u8::from_str_radix(hex, 16)
                    .map_err(|_| AppError::Validation("Bad path encoding".into()))?,
            );
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| AppError::Validation("The path is not valid text".into()))
}

/// The path to write to, or why it is not acceptable.
pub fn checked_export_path(raw: &str) -> AppResult<PathBuf> {
    let path = PathBuf::from(raw);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !path.is_absolute()
        || !extension.is_some_and(|extension| ALLOWED_EXTENSIONS.contains(&extension.as_str()))
    {
        return Err(AppError::Validation(
            "Choose a file ending in .svg or .png".into(),
        ));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(AppError::Validation("The folder does not exist".into()));
    }
    Ok(path)
}

/// Writes the bytes sent as the request body to the file named in the `path` header (percent-encoded).
#[tauri::command]
pub async fn write_export_file(request: Request<'_>) -> AppResult<u64> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::Validation("Expected the file's bytes".into()));
    };
    let header = request
        .headers()
        .get(PATH_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Validation("No file path was given".into()))?;
    let path = checked_export_path(&percent_decode(header)?)?;
    tokio::fs::write(&path, bytes).await?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_escapes_are_decoded_as_utf8() {
        assert_eq!(
            percent_decode("%2FUsers%2Fa%20b%2Fcaf%C3%A9.svg").unwrap(),
            "/Users/a b/café.svg"
        );
        assert_eq!(percent_decode("plain").unwrap(), "plain");
        assert!(percent_decode("%2").is_err());
        assert!(percent_decode("%zz").is_err());
        assert!(percent_decode("%ff%fe").is_err());
    }

    #[test]
    fn only_picture_files_in_an_existing_folder_can_be_written() {
        let folder = std::env::temp_dir();

        assert!(checked_export_path(&format!("{}/diagram.svg", folder.display())).is_ok());
        assert!(checked_export_path(&format!("{}/DIAGRAM.PNG", folder.display())).is_ok());
        for bad in [
            "relative/diagram.svg",
            "/tmp/diagram.sh",
            "/tmp/diagram",
            "/etc/passwd",
            "/no/such/folder/diagram.svg",
            "",
        ] {
            assert!(checked_export_path(bad).is_err(), "{bad}");
        }
    }
}
