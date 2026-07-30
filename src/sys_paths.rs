use std::path::PathBuf;
use std::process::{Command, Stdio};

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
            .args(["-c", "import sys;print(*sys.path,sep='\\0',end='')"])
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
        let stdout = match String::from_utf8(output.stdout) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if stdout.is_empty() {
            continue;
        }
        let nul_separated = stdout.contains('\0');
        for raw_token in stdout.split('\0') {
            let token = if nul_separated {
                raw_token
            } else {
                raw_token.trim_end_matches(['\r', '\n'])
            };
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
