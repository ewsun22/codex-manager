//! Explicit, ignored integration observation of the reviewed prebuilt core.
//! No real OAuth material, Keychain, provider or user configuration is used.

use super::*;
use std::os::unix::fs::PermissionsExt;

pub(super) async fn verify_isolated_core(data_dir: &Path) {
    let baseline = pinned_core_asset().unwrap();
    let metadata = read_core_metadata(data_dir).unwrap();
    assert!(reviewed_core_ready(data_dir, &metadata, &baseline));
    let runtime = runtime_dir(data_dir).join("isolated-observation");
    let auth_dir = runtime.join("auth");
    for directory in [&runtime, &auth_dir, &runtime.join("logs")] {
        ensure_private_dir(directory).unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = Zeroizing::new(Uuid::new_v4().to_string());
    let config_path = runtime.join("config.yaml");
    atomic_private_write(
        &config_path,
        oauth_sidecar_config(&token, port, &auth_dir, &runtime).as_bytes(),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let core = core_dir(data_dir);
    let binary = find_unique_core_binary(&core).unwrap();
    drop(listener);
    let child = Command::new(binary)
        .args(["-config", config_path.to_str().unwrap(), "-local-model"])
        .current_dir(&core)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    // Own the actual child immediately so all assertion failures reap it.
    let mut handle = ServerHandle {
        port,
        source: "isolated-empty-auth".into(),
        provider_id: None,
        provider_name: "人工空凭据池".into(),
        endpoint: format!("http://127.0.0.1:{port}/v1"),
        started_at: Utc::now().to_rfc3339(),
        token,
        sidecar: Some(SidecarProcess {
            child: Mutex::new(child),
            data_dir: data_dir.into(),
            runtime_dir: runtime.clone(),
        }),
        proxy_auth_store: None,
        proxy_auth_runtime: None,
    };
    wait_for_health(&mut handle).await.unwrap();
    let client = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let endpoint = format!("http://127.0.0.1:{port}");
    let unauthenticated = client
        .get(format!("{endpoint}/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let models = client
        .get(format!("{endpoint}/v1/models"))
        .bearer_auth(handle.token.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(models.status(), reqwest::StatusCode::OK);
    let management = client
        .get(format!("{endpoint}/v0/management/config"))
        .bearer_auth(handle.token.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(management.status(), reqwest::StatusCode::NOT_FOUND);
    let panel = client
        .get(format!("{endpoint}/management.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(panel.status(), reqwest::StatusCode::NOT_FOUND);

    // The empty auth directory cannot authenticate to an upstream. A unique,
    // artificial marker lets us detect accidental request-body file logging.
    let marker = format!("synthetic-body-observation-{}", Uuid::new_v4());
    let rejected = client
        .post(format!("{endpoint}/v1/responses"))
        .bearer_auth(handle.token.as_str())
        .json(&serde_json::json!({"model": "qa-no-provider", "input": marker}))
        .send()
        .await
        .unwrap();
    let rejected_status = rejected.status();
    assert!(rejected_status.is_client_error() || rejected_status.is_server_error());
    let malformed = client
        .post(format!("{endpoint}/v1/responses"))
        .bearer_auth(handle.token.as_str())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{invalid synthetic JSON")
        .send()
        .await
        .unwrap();
    let malformed_status = malformed.status();
    assert!(malformed_status.is_client_error() || malformed_status.is_server_error());

    let sockets = Command::new("/usr/sbin/lsof")
        .args([
            "-nP",
            "-a",
            "-p",
            &pid.to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
            "-Fn",
        ])
        .output()
        .unwrap();
    assert!(sockets.status.success());
    let sockets = String::from_utf8(sockets.stdout).unwrap();
    let addresses: Vec<_> = sockets
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .collect();
    assert_eq!(addresses, [format!("127.0.0.1:{port}")]);
    assert_eq!(fs::read_dir(&auth_dir).unwrap().count(), 0);
    let mut inspected_files = 0;
    for entry in walkdir::WalkDir::new(data_dir).follow_links(false) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            assert!(
                !bytes
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes()),
                "人工请求标记不得写入内核文件或日志"
            );
            inspected_files += 1;
        }
    }
    handle.stop().await.unwrap();
    assert!(!runtime.exists());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    println!(
        "isolated-core-observation={}",
        serde_json::json!({
            "version": baseline.version,
            "asset": baseline.name,
            "sha256": baseline.sha256,
            "health": "passed",
            "unauthenticated_status": 401,
            "authenticated_models_status": 200,
            "management_status": 404,
            "control_panel_status": 404,
            "empty_pool_request_status": rejected_status.as_u16(),
            "malformed_request_status": malformed_status.as_u16(),
            "listener": "IPv4 loopback only",
            "request_marker_on_disk": false,
            "inspected_files": inspected_files,
            "real_oauth_used": false,
            "runtime_removed": true,
            "port_released": true
        })
    );
}
