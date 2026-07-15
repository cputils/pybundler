use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::resolver::is_importable_zip;

/// Discovers Python `sys.path` entries.
///
/// All interpreters are spawned concurrently, then their outputs are
/// collected in the original order. Each interpreter prints its `sys.path`
/// entries space-separated. Returns absolute directories and importable
/// zip/egg files. Errors are silently ignored.
pub(crate) fn discover_sys_paths(interpreters: &[String]) -> Vec<PathBuf> {
    let mut children = Vec::with_capacity(interpreters.len());
    for interpreter in interpreters {
        let child = Command::new(interpreter)
            .args(["-c", "import sys;print(*sys.path)"])
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
        for token in stdout.split_whitespace() {
            if token.is_empty() {
                continue;
            }
            let path = PathBuf::from(token);
            if path.is_absolute()
                && !paths.contains(&path)
                && (path.is_dir() || is_importable_zip(&path))
            {
                paths.push(path);
            }
        }
    }
    paths
}
