//! Black-box contracts for authenticated metadata synchronization, without inference.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "cli_contract/support.rs"]
mod support;

#[test]
fn sync_caches_authenticated_metadata_and_prices_without_rewriting_configuration() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let user_dir = home.path().join(".smith");
    std::fs::create_dir(&user_dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let config = format!(
        r#"default_profile = "code"
[model_sync]
enabled = false
[profiles.code]
provider = "xiaoge"
model = "selected-large"
[providers.xiaoge]
kind = "openai-compatible"
base_url = {endpoint:?}
api_key = "fixture-listing-secret"
"#
    );
    let config_path = user_dir.join("config.toml");
    std::fs::write(&config_path, &config).unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(12);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "model sync made no listing request"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accepting listing request: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0 && request.len() + count <= 8192);
            request.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
        assert!(request.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == "Bearer fixture-listing-secret"
            })
        }));
        let body = r#"{"data":[{"id":"selected-large","name":"Selected large","limits":{"context":1048576,"input":1048576,"output":256000},"capabilities":{"tool_calls":true},"input_modalities":["text","image"],"output_modalities":["text"],"cost":{"input":1,"output":2},"internal_note":"must-not-cache-me"}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
        command
            .args(args)
            .current_dir(project.path())
            .env_clear()
            .env("HOME", home.path())
            .env("TMPDIR", std::env::temp_dir());
        support::run_command(
            &mut command,
            support::Deadline::after(Duration::from_secs(25)),
        )
    };
    let synced = run(&["models", "sync", "--provider", "xiaoge", "--force"]);
    assert!(
        synced.status.success(),
        "{}",
        String::from_utf8_lossy(&synced.stderr)
    );
    server.join().unwrap();
    assert!(!String::from_utf8_lossy(&synced.stdout).contains("fixture-listing-secret"));
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), config);
    let cache = std::fs::read_dir(user_dir.join("cache/provider-models-v1"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("one normalized endpoint cache");
    let bytes = std::fs::read(cache.path()).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains("fixture-listing-secret"));
    assert!(!text.contains("must-not-cache-me"));
    assert!(text.contains("selected-large"));
    assert!(text.contains("256000"));
    assert_eq!(
        cache.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    let cached = run(&["models", "sync", "--provider", "xiaoge"]);
    assert!(
        cached.status.success(),
        "{}",
        String::from_utf8_lossy(&cached.stderr)
    );
    assert_eq!(
        std::fs::read(cache.path()).unwrap(),
        bytes,
        "fresh metadata needs no network request"
    );
    let explained = run(&[
        "config",
        "explain",
        "input_modalities",
        "--provider",
        "xiaoge",
        "--model",
        "selected-large",
    ]);
    assert!(
        explained.status.success(),
        "{}",
        String::from_utf8_lossy(&explained.stderr)
    );
    assert!(String::from_utf8_lossy(&explained.stdout).contains("image"));
}
