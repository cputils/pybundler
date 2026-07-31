use std::fs;
use std::process::Command;

#[test]
fn bundles_to_standard_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_pybundler"))
        .args(["testdata/import-syntax/main.py", "--no-tree-shaking"])
        .output()
        .expect("run pybundler");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("CLI output should be UTF-8");
    assert!(stdout.contains("if __name__ == \"main\""));
}

#[test]
fn bundles_to_output_file() {
    let output_path = std::env::temp_dir().join(format!(
        "pybundler-cli-test-{}-bundled.py",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_pybundler"))
        .args([
            "testdata/import-syntax/main.py",
            "--no-tree-shaking",
            "--output",
        ])
        .arg(&output_path)
        .output()
        .expect("run pybundler");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let bundled = fs::read_to_string(&output_path).expect("read bundled output");
    fs::remove_file(output_path).expect("remove bundled output");
    assert!(bundled.contains("if __name__ == \"main\""));
}

#[test]
fn reports_bundling_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_pybundler"))
        .arg("missing.py")
        .output()
        .expect("run pybundler");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("CLI error should be UTF-8");
    assert!(stderr.starts_with("pybundler: "));
    assert!(stderr.contains("entry file"));
}
