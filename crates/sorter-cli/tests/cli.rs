use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn init_validate_and_dry_run_work_end_to_end() {
    let root = tempdir().unwrap();
    let config = root.path().join("sorter.toml");
    let database = root.path().join("sorter.db");
    let binary = env!("CARGO_BIN_EXE_sorter");

    let status = Command::new(binary)
        .args(["--config", config.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(binary)
        .args(["--config", config.to_str().unwrap(), "validate"])
        .status()
        .unwrap();
    assert!(status.success());

    let incoming = root.path().join("incoming");
    fs::create_dir(&incoming).unwrap();
    fs::write(incoming.join("report.pdf"), b"content").unwrap();
    let output = Command::new(binary)
        .args([
            "--config",
            config.to_str().unwrap(),
            "--database",
            database.to_str().unwrap(),
            "run",
            incoming.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["matched"], 1);
    assert!(incoming.join("report.pdf").exists());
    assert!(!database.exists(), "dry-run must not create history");
}
