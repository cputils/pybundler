use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use crate::resolver::is_importable_zip;

/// Discovers Python `sys.path` entries.
///
/// All interpreters are spawned concurrently, then their outputs are
/// collected in the original order. Each interpreter prints its `sys.path`
/// entries NUL-separated. Returns absolute directories and ZIP-based path
/// entries. Errors are silently ignored.
pub(crate) fn discover_sys_paths(interpreters: &[String]) -> Vec<PathBuf> {
    let current_dir = std::env::current_dir().ok();
    let mut children = Vec::with_capacity(interpreters.len());
    for interpreter in interpreters {
        let child = Command::new(interpreter)
            .args([
                "-c",
                "import os,sys;sys.stdout.buffer.write(b'\\0'.join(map(os.fsencode,sys.path)))",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        children.push(child);
    }

    let mut paths = Vec::new();
    for child_result in children {
        let output = match child_result.and_then(|c| c.wait_with_output()) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        if output.stdout.is_empty() {
            continue;
        }
        for token in sys_path_tokens(output.stdout) {
            let path = if token.is_empty() {
                let Some(current_dir) = &current_dir else {
                    continue;
                };
                current_dir.clone()
            } else {
                let path = PathBuf::from(token);
                if path.is_absolute() {
                    path
                } else {
                    let Some(current_dir) = &current_dir else {
                        continue;
                    };
                    current_dir.join(path)
                }
            };
            if !paths.contains(&path) && (path.is_dir() || is_importable_zip(&path)) {
                paths.push(path);
            }
        }
    }
    paths
}

#[cfg(unix)]
fn sys_path_tokens(stdout: Vec<u8>) -> Vec<OsString> {
    let nul_separated = stdout.contains(&0);
    stdout
        .split(|byte| *byte == 0)
        .map(|token| {
            let token = if nul_separated {
                token
            } else {
                let end = token
                    .iter()
                    .rposition(|byte| !matches!(byte, b'\r' | b'\n'))
                    .map_or(0, |index| index + 1);
                &token[..end]
            };
            OsString::from_vec(token.to_vec())
        })
        .collect()
}

#[cfg(not(unix))]
fn sys_path_tokens(stdout: Vec<u8>) -> Vec<OsString> {
    let Ok(stdout) = String::from_utf8(stdout) else {
        return Vec::new();
    };
    let nul_separated = stdout.contains('\0');
    stdout
        .split('\0')
        .map(|token| {
            if nul_separated {
                token
            } else {
                token.trim_end_matches(['\r', '\n'])
            }
        })
        .map(OsString::from)
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::sys_path_tokens;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn preserves_non_utf8_sys_path_entries() {
        let tokens = sys_path_tokens(b"/tmp/non-utf8-\xff\0/tmp/other".to_vec());

        assert_eq!(tokens[0].as_bytes(), b"/tmp/non-utf8-\xff");
        assert_eq!(tokens[1].as_bytes(), b"/tmp/other");
    }
}
