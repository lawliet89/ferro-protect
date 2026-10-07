//! `ferro-protect files …` subcommands. `upload` lands in phase 10.

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use ferro_protect::ProtectClient;
use ferro_protect::models::{AssetFile, AssetFileType};

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every device asset file of one type.
    List {
        /// Asset file type. Validated locally because the NVR answers
        /// an unknown type with an empty list rather than an error.
        #[arg(value_enum)]
        file_type: FileTypeArg,
    },
}

/// CLI-facing file type enum. Maps 1:1 onto [`AssetFileType`] but
/// kept separate so we can keep `clap::ValueEnum` derivation off the
/// library type.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FileTypeArg {
    Animations,
}

impl From<FileTypeArg> for AssetFileType {
    fn from(value: FileTypeArg) -> Self {
        match value {
            FileTypeArg::Animations => Self::Animations,
        }
    }
}

/// Dispatch `files` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List { file_type } => {
            let file_type = AssetFileType::from(file_type);
            let files = client
                .files()
                .list(file_type)
                .await
                .with_context(|| format!("listing {file_type} files"))?;
            output::emit_stdout(&files, json, || render_table(&files))?;
        }
    }
    Ok(())
}

fn render_table(files: &[AssetFile]) -> String {
    if files.is_empty() {
        return "(no files)\n".to_string();
    }
    let headers = &["NAME", "ORIGINAL NAME", "TYPE", "PATH"];
    let rows: Vec<Vec<String>> = files
        .iter()
        .map(|f| {
            vec![
                f.name.to_string(),
                output::display_optional(f.original_name.as_deref()),
                f.type_.to_string(),
                f.path.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}
