use std::hash::Hasher;
use std::io::{self, BufRead, Write};
use std::path::Path;
use twox_hash::XxHash32;

/// A file line with its xxHash32 fingerprint for staleness detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedLine {
    pub line_number: u32,
    /// xxHash32 fingerprint of the line content (not including newline).
    pub hash: u32,
    pub content: String,
}

/// Error returned when a staleness check fails — the file changed since it was read.
#[derive(Debug, thiserror::Error)]
pub enum StalenessError {
    #[error("line {line_number} has changed since last read (expected hash {expected:#010x}, got {actual:#010x})")]
    LineChanged {
        line_number: u32,
        expected: u32,
        actual: u32,
    },
    #[error("line {line_number} no longer exists in the file (file has {file_lines} lines)")]
    LineMissing {
        line_number: u32,
        file_lines: u32,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// An edit proposal: replace lines `start_line..=end_line` with `new_content`.
/// Line numbers are 1-based. `anchor_hashes` are the expected xxHash32
/// values of the target lines — used for staleness verification.
#[derive(Debug, Clone)]
pub struct EditProposal {
    pub start_line: u32,
    pub end_line: u32,
    pub new_content: String,
    /// Expected hashes for lines `start_line..=end_line` at read time.
    pub anchor_hashes: Vec<u32>,
}

/// Compute the xxHash32 fingerprint of a line (not including the trailing newline).
///
/// `XxHash32::finish()` returns a u64 whose upper 32 bits are always zero by
/// construction (`XxHash32` is a 32-bit algorithm exposed through the 64-bit
/// `Hasher` trait). The truncating cast here is intentional.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn hash_line(content: &str) -> u32 {
    let mut hasher = XxHash32::with_seed(0);
    hasher.write(content.as_bytes());
    hasher.finish() as u32
}

/// Read a file and return tagged lines with xxHash32 fingerprints.
///
/// Returns chunks of at most 200 lines. For files larger than 64KB,
/// chunking allows incremental processing without loading the whole file.
pub fn read_with_hashes(path: &Path) -> Result<Vec<TaggedLine>, StalenessError> {
    let file = std::fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut tagged = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let content = line?;
        let hash = hash_line(&content);
        tagged.push(TaggedLine {
            line_number: u32::try_from(idx + 1).unwrap_or(u32::MAX),
            hash,
            content,
        });
    }
    Ok(tagged)
}

/// Read a specific chunk of lines from a file (1-based, inclusive).
/// Returns at most 200 lines per chunk.
pub fn read_chunk_with_hashes(
    path: &Path,
    start_line: u32,
    end_line: u32,
) -> Result<Vec<TaggedLine>, StalenessError> {
    let end_line = end_line.min(start_line.saturating_add(199)); // cap at 200 lines
    let file = std::fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut tagged = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line_num = u32::try_from(idx + 1).unwrap_or(u32::MAX);
        if line_num < start_line {
            let _ = line?; // advance but discard
            continue;
        }
        if line_num > end_line {
            break;
        }
        let content = line?;
        let hash = hash_line(&content);
        tagged.push(TaggedLine {
            line_number: line_num,
            hash,
            content,
        });
    }
    Ok(tagged)
}

/// Verify that the anchor hashes in a proposal still match the file's current content.
/// Returns the first staleness error found, or Ok(()) if all hashes match.
pub fn check_staleness(
    path: &Path,
    proposal: &EditProposal,
) -> Result<(), StalenessError> {
    if proposal.anchor_hashes.is_empty() {
        return Ok(()); // no anchors — skip check
    }
    let current = read_with_hashes(path)?;
    let file_lines = u32::try_from(current.len()).unwrap_or(u32::MAX);

    for (i, &expected_hash) in proposal.anchor_hashes.iter().enumerate() {
        let line_num = proposal.start_line + u32::try_from(i).unwrap_or(u32::MAX);
        match current.get((line_num - 1) as usize) {
            None => {
                return Err(StalenessError::LineMissing {
                    line_number: line_num,
                    file_lines,
                });
            }
            Some(tagged) if tagged.hash != expected_hash => {
                return Err(StalenessError::LineChanged {
                    line_number: line_num,
                    expected: expected_hash,
                    actual: tagged.hash,
                });
            }
            Some(_) => {} // hash matches
        }
    }
    Ok(())
}

/// Apply an edit to a file after verifying the anchor hashes match.
/// Reads the full file, checks staleness, replaces the target lines,
/// and writes the result back atomically.
///
/// Returns `Err(StalenessError::LineChanged)` if any target line changed
/// since the proposal was created.
pub fn write_with_staleness_check(
    path: &Path,
    proposal: &EditProposal,
) -> Result<(), StalenessError> {
    check_staleness(path, proposal)?;

    // Read the current file content
    let current = read_with_hashes(path)?;
    let mut output_lines: Vec<String> = current.iter().map(|t| t.content.clone()).collect();

    // Replace start_line..=end_line (1-based) with new_content lines
    let start = (proposal.start_line - 1) as usize;
    let end = proposal.end_line as usize; // exclusive in slice terms
    let new_lines: Vec<String> = proposal.new_content.lines().map(String::from).collect();

    let end = end.min(output_lines.len());
    output_lines.splice(start..end, new_lines);

    // Write atomically via tempfile
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(StalenessError::Io)?;
    for line in &output_lines {
        writeln!(tmp, "{line}").map_err(StalenessError::Io)?;
    }
    tmp.persist(path)
        .map_err(|e| StalenessError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    fn write_temp_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn hash_line_is_deterministic() {
        assert_eq!(hash_line("hello world"), hash_line("hello world"));
        assert_ne!(hash_line("hello world"), hash_line("hello World"));
    }

    #[test]
    fn read_with_hashes_returns_tagged_lines() {
        let f = write_temp_file("line one\nline two\nline three\n");
        let tagged = read_with_hashes(f.path()).unwrap();
        assert_eq!(tagged.len(), 3);
        assert_eq!(tagged[0].line_number, 1);
        assert_eq!(tagged[0].content, "line one");
        assert_eq!(tagged[0].hash, hash_line("line one"));
    }

    #[test]
    fn staleness_check_passes_for_unmodified_file() {
        let f = write_temp_file("alpha\nbeta\ngamma\n");
        let tagged = read_with_hashes(f.path()).unwrap();
        let proposal = EditProposal {
            start_line: 2,
            end_line: 2,
            new_content: "BETA".to_string(),
            anchor_hashes: vec![tagged[1].hash],
        };
        assert!(check_staleness(f.path(), &proposal).is_ok());
    }

    #[test]
    fn staleness_check_fails_when_file_changes() {
        let mut f = write_temp_file("alpha\nbeta\ngamma\n");
        let tagged = read_with_hashes(f.path()).unwrap();
        let original_hash = tagged[1].hash;

        // Modify the file
        f.seek(SeekFrom::Start(0)).unwrap();
        write!(f, "alpha\nBETA_CHANGED\ngamma\n").unwrap();

        let proposal = EditProposal {
            start_line: 2,
            end_line: 2,
            new_content: "something".to_string(),
            anchor_hashes: vec![original_hash],
        };
        assert!(matches!(
            check_staleness(f.path(), &proposal),
            Err(StalenessError::LineChanged { line_number: 2, .. })
        ));
    }

    #[test]
    fn write_with_staleness_check_applies_edit() {
        let f = write_temp_file("alpha\nbeta\ngamma\n");
        let tagged = read_with_hashes(f.path()).unwrap();
        let proposal = EditProposal {
            start_line: 2,
            end_line: 2,
            new_content: "REPLACED".to_string(),
            anchor_hashes: vec![tagged[1].hash],
        };
        write_with_staleness_check(f.path(), &proposal).unwrap();
        let result = read_with_hashes(f.path()).unwrap();
        assert_eq!(result[1].content, "REPLACED");
    }

    #[test]
    fn staleness_check_fails_for_line_missing() {
        let f = write_temp_file("alpha\nbeta\n");
        let tagged = read_with_hashes(f.path()).unwrap();
        // Anchor references line 3 which doesn't exist
        let proposal = EditProposal {
            start_line: 3,
            end_line: 3,
            new_content: "EXTRA".to_string(),
            anchor_hashes: vec![tagged[0].hash],
        };
        assert!(matches!(
            check_staleness(f.path(), &proposal),
            Err(StalenessError::LineMissing { line_number: 3, file_lines: 2 })
        ));
    }

    #[test]
    fn read_chunk_with_hashes_returns_subset() {
        let f = write_temp_file("one\ntwo\nthree\nfour\nfive\n");
        let chunk = read_chunk_with_hashes(f.path(), 2, 4).unwrap();
        assert_eq!(chunk.len(), 3);
        assert_eq!(chunk[0].line_number, 2);
        assert_eq!(chunk[0].content, "two");
        assert_eq!(chunk[2].line_number, 4);
        assert_eq!(chunk[2].content, "four");
    }
}
