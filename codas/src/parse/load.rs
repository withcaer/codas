//! Filesystem loader for codas that use other codas.
use std::path::{Path, PathBuf};

use super::{parse_with, ParseError};
use crate::types::Coda;

/// Loads and parses the coda at `path`, recursively
/// loading every coda it uses relative to its directory.
///
/// Returns the coda alongside the paths of every file
/// that was loaded, beginning with `path`.
pub fn load(path: &Path) -> Result<(Coda, Vec<PathBuf>), ParseError> {
    let mut sources = vec![];
    let coda = load_into(path, &mut vec![], &mut sources)?;
    Ok((coda, sources))
}

/// Loads the coda at `path`, tracking the `stack` of
/// codas currently being loaded to detect cycles.
fn load_into(
    path: &Path,
    stack: &mut Vec<PathBuf>,
    sources: &mut Vec<PathBuf>,
) -> Result<Coda, ParseError> {
    let io = |error: std::io::Error| ParseError::Io {
        path: path.display().to_string().into(),
        reason: error.to_string().into(),
    };
    let canonical = path.canonicalize().map_err(io)?;
    if stack.contains(&canonical) {
        return Err(ParseError::CyclicUse {
            path: path.display().to_string().into(),
        });
    }
    let markdown = std::fs::read_to_string(path).map_err(io)?;
    sources.push(path.to_path_buf());

    let dir = path.parent().unwrap_or(Path::new(""));
    stack.push(canonical);
    let coda = parse_with(&markdown, |used| load_into(&dir.join(used), stack, sources));
    stack.pop();
    coda
}
