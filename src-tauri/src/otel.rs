//! Authenticated, TLS-protected, loopback-only OTLP/HTTP receiver.
//!
//! OTLP payloads are untrusted. Authentication and request admission happen
//! before body extraction. Log bodies, request headers, and the bearer secret
//! are never materialized into the storage projection or logs.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    future::{Future, Ready, ready},
    io::{self, Cursor, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_server::{
    Handle as AxumServerHandle,
    accept::Accept,
    tls_rustls::{RustlsAcceptor, RustlsConfig},
};
use chrono::{DateTime, Utc};
use codex_core::TokenUsage;
use codex_storage::{OtelMetadata, Store};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest},
    common::v1::{KeyValue, any_value::Value as AnyValue},
    logs::v1::ResourceLogs,
    metrics::v1::{Metric, ResourceMetrics, metric::Data as MetricData},
};
use prost::Message;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant as TokioInstant, Sleep},
};
use url::Url;
use uuid::Uuid;

const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_TLS_ASSET_BYTES: u64 = 64 * 1024;
const MAX_RESOURCES: usize = 64;
const MAX_SCOPES: usize = 256;
const MAX_EVENTS: usize = codex_storage::MAX_OTEL_BATCH_EVENTS;
const MAX_ATTRIBUTES: usize = 8_192;
const MAX_ATTRIBUTES_PER_ITEM: usize = 128;
const MAX_DATA_POINTS: usize = 4_096;
const MAX_EVENT_NAME_BYTES: usize = 256;
const MAX_SCALAR_BYTES: usize = 512;
const MAX_SCHEMA_STRING_BYTES: usize = 512;
const MAX_DURATION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_RESPONSE_BYTES: i64 = 1_099_511_627_776;
const MAX_TOKEN_COUNT: i64 = TokenUsage::MAX_CALL_FIELD_VALUE;
const MAX_CONCURRENT_REQUESTS: usize = 4;
const MAX_CONCURRENT_CONNECTIONS: usize = 16;
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUESTS_PER_WINDOW: u32 = 120;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const PROCESSING_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PAST_TIMESTAMP_DRIFT: chrono::Duration = chrono::Duration::hours(24);
const MAX_FUTURE_TIMESTAMP_DRIFT: chrono::Duration = chrono::Duration::minutes(5);
const TOKEN_HEADER: &str = "x-codex-manager-token";
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_BYTES: usize = TOKEN_BYTES * 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedEvent {
    pub id: String,
    pub signal: String,
    pub event_name: Option<String>,
    pub occurred_at: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub response_bytes: Option<i64>,
    pub endpoint: Option<String>,
    pub success: Option<bool>,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub attributes_json: String,
}

/// Running receiver metadata. The credential has no getter and is disclosed
/// only when the caller explicitly asks for the complete config snippet.
pub struct ServerHandle {
    port: u16,
    endpoint: String,
    certificate_path: PathBuf,
    token: String,
    server: AxumServerHandle,
}

impl ServerHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn certificate_path(&self) -> &Path {
        &self.certificate_path
    }

    pub fn config_snippet(&self) -> String {
        let logs_endpoint = toml_string(&format!("{}/v1/logs", self.endpoint));
        let metrics_endpoint = toml_string(&format!("{}/v1/metrics", self.endpoint));
        let token = toml_string(&self.token);
        let certificate = toml_string(&self.certificate_path.to_string_lossy());
        format!(
            "[otel]\nlog_user_prompt = false\nexporter = {{ otlp-http = {{ endpoint = {logs_endpoint}, protocol = \"binary\", headers = {{ \"{TOKEN_HEADER}\" = {token} }}, tls = {{ ca-certificate = {certificate} }} }} }}\nmetrics_exporter = {{ otlp-http = {{ endpoint = {metrics_endpoint}, protocol = \"binary\", headers = {{ \"{TOKEN_HEADER}\" = {token} }}, tls = {{ ca-certificate = {certificate} }} }} }}"
        )
    }

    pub fn stop(self) {
        self.server.graceful_shutdown(Some(Duration::from_secs(2)));
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.server.graceful_shutdown(Some(Duration::from_secs(2)));
    }
}

#[derive(Clone)]
struct ReceiverState {
    database_path: PathBuf,
    secret: Arc<ReceiverSecret>,
    receiver_identity: Arc<str>,
    admission: Arc<Semaphore>,
    rate: Arc<Mutex<RateWindow>>,
}

#[derive(Clone)]
struct ConnectionLimitAcceptor {
    permits: Arc<Semaphore>,
    idle_timeout: Duration,
}

struct ConnectionGuard<T> {
    inner: T,
    _permit: OwnedSemaphorePermit,
    idle_timeout: Duration,
    idle_deadline: Pin<Box<Sleep>>,
}

impl<T> ConnectionGuard<T> {
    fn new(inner: T, permit: OwnedSemaphorePermit, idle_timeout: Duration) -> Self {
        Self {
            inner,
            _permit: permit,
            idle_timeout,
            idle_deadline: Box::pin(tokio::time::sleep(idle_timeout)),
        }
    }

    fn reset_idle_deadline(&mut self) {
        self.idle_deadline
            .as_mut()
            .reset(TokioInstant::now() + self.idle_timeout);
    }

    fn poll_idle_deadline(&mut self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.idle_deadline.as_mut().poll(context) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OTLP receiver connection idle timeout",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Accept<TcpStream, S> for ConnectionLimitAcceptor {
    type Stream = ConnectionGuard<TcpStream>;
    type Service = S;
    type Future = Ready<io::Result<(Self::Stream, Self::Service)>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let permit = self.permits.clone().try_acquire_owned().map_err(|_| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "OTLP receiver connection limit reached",
            )
        });
        ready(permit.map(|permit| {
            (
                ConnectionGuard::new(stream, permit, self.idle_timeout),
                service,
            )
        }))
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for ConnectionGuard<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Poll::Ready(error) = self.poll_idle_deadline(context) {
            return Poll::Ready(error);
        }
        let before = buffer.filled().len();
        match Pin::new(&mut self.inner).poll_read(context, buffer) {
            Poll::Ready(Ok(())) if buffer.filled().len() > before => {
                self.reset_idle_deadline();
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for ConnectionGuard<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Poll::Ready(error) = self.poll_idle_deadline(context) {
            return Poll::Ready(error.map(|()| 0));
        }
        match Pin::new(&mut self.inner).poll_write(context, buffer) {
            Poll::Ready(Ok(written)) if written > 0 => {
                self.reset_idle_deadline();
                Poll::Ready(Ok(written))
            }
            result => result,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Poll::Ready(error) = self.poll_idle_deadline(context) {
            return Poll::Ready(error);
        }
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

struct ReceiverSecret([u8; TOKEN_HEX_BYTES]);

impl ReceiverSecret {
    fn from_token(token: &str) -> Result<Self, String> {
        let bytes: [u8; TOKEN_HEX_BYTES] = token
            .as_bytes()
            .try_into()
            .map_err(|_| "OTLP receiver token length is invalid".to_string())?;
        Ok(Self(bytes))
    }

    fn matches(&self, candidate: &[u8]) -> bool {
        let mut normalized = [0_u8; TOKEN_HEX_BYTES];
        let copy_length = candidate.len().min(TOKEN_HEX_BYTES);
        normalized[..copy_length].copy_from_slice(&candidate[..copy_length]);
        let same_length = subtle::Choice::from((candidate.len() == TOKEN_HEX_BYTES) as u8);
        bool::from(same_length & normalized.ct_eq(&self.0))
    }
}

struct RateWindow {
    started: Instant,
    accepted: u32,
}

impl RateWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            accepted: 0,
        }
    }

    fn admit(&mut self) -> bool {
        if self.started.elapsed() >= RATE_WINDOW {
            self.started = Instant::now();
            self.accepted = 0;
        }
        if self.accepted >= MAX_REQUESTS_PER_WINDOW {
            return false;
        }
        self.accepted += 1;
        true
    }
}

struct TlsAssets {
    certificate_path: PathBuf,
    certificate_pem: Vec<u8>,
    private_key_pem: Vec<u8>,
}

/// Starts a TLS-only receiver on a persistent IPv4 loopback port. The caller
/// supplies the app-private data directory used for the receiver identity.
pub fn start(database_path: PathBuf, data_dir: PathBuf) -> Result<ServerHandle, String> {
    let assets = ensure_tls_assets(&data_dir)?;
    let tls = validated_rustls_config(&assets.certificate_pem, &assets.private_key_pem)?;
    let (listener, port) = bind_persistent_listener(&data_dir)?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("无法配置 OTLP listener：{error}"))?;

    // Derive the credential from the app-private 0600 key instead of writing a
    // second secret file. The token remains stable across app restarts, while
    // another OS account cannot read the derivation input.
    let token = derive_receiver_token(&assets.private_key_pem);
    let secret = Arc::new(ReceiverSecret::from_token(&token)?);
    let mut identity_digest = Sha256::new();
    identity_digest.update(b"codex-manager-otel-receiver-v1\0");
    identity_digest.update(&assets.certificate_pem);
    let receiver_identity: Arc<str> = hex::encode(identity_digest.finalize()).into();
    let state = ReceiverState {
        database_path,
        secret,
        receiver_identity,
        admission: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        rate: Arc::new(Mutex::new(RateWindow::new())),
    };
    let app = receiver_router(state);
    let server = AxumServerHandle::new();
    let server_for_task = server.clone();
    tauri::async_runtime::spawn(async move {
        let acceptor = RustlsAcceptor::new(tls)
            .handshake_timeout(PROCESSING_TIMEOUT)
            .acceptor(ConnectionLimitAcceptor {
                permits: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
                idle_timeout: CONNECTION_IDLE_TIMEOUT,
            });
        let _ = axum_server::from_tcp(listener)
            .acceptor(acceptor)
            .handle(server_for_task)
            .serve(app.into_make_service())
            .await;
    });

    Ok(ServerHandle {
        port,
        endpoint: format!("https://localhost:{port}"),
        certificate_path: assets.certificate_path,
        token,
        server,
    })
}

fn bind_persistent_listener(data_dir: &Path) -> Result<(TcpListener, u16), String> {
    let directory = data_dir.join("otel");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建 OTLP 数据目录：{error}"))?;
    set_private_mode(&directory, 0o700)?;
    let port_path = directory.join("receiver-port");
    let persisted = read_receiver_port(&port_path)?;
    let requested_port = persisted.unwrap_or(0);
    let listener = TcpListener::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        requested_port,
    ))
    .map_err(|error| match persisted {
        Some(port) => format!(
            "无法绑定已保存的本机 OTLP 端口 {port}：{error}。为避免 Codex exporter 配置静默失效，接收器未换用其他端口"
        ),
        None => format!("无法分配本机 OTLP 接收器端口：{error}"),
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法读取 OTLP listener 地址：{error}"))?
        .port();
    if persisted.is_none() {
        atomic_write_private(&port_path, port.to_string().as_bytes())?;
    }
    Ok((listener, port))
}

fn read_receiver_port(path: &Path) -> Result<Option<u16>, String> {
    let Some(content) = read_regular_bounded(path)? else {
        return Ok(None);
    };
    if content.is_empty() || content.len() > 5 || !content.iter().all(u8::is_ascii_digit) {
        return Err("OTLP receiver-port 文件格式无效".into());
    }
    let text = std::str::from_utf8(&content)
        .map_err(|_| "OTLP receiver-port 文件不是 ASCII 整数".to_string())?;
    let port = text
        .parse::<u16>()
        .map_err(|_| "OTLP receiver-port 文件不在有效端口范围内".to_string())?;
    if port == 0 || port.to_string().as_bytes() != content {
        return Err("OTLP receiver-port 文件不是规范的非零端口".into());
    }
    Ok(Some(port))
}

fn receiver_router(state: ReceiverState) -> Router {
    Router::new()
        .route("/v1/logs", post(receive_logs))
        .route("/v1/metrics", post(receive_metrics))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_and_admit,
        ))
        .with_state(state)
}

/// Shared security boundary for every receiver route. This middleware runs
/// before the `Bytes` extractor, so unauthenticated requests are never decoded.
async fn authenticate_and_admit(
    State(state): State<ReceiverState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.headers().contains_key("origin") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let authenticated = request
        .headers()
        .get(TOKEN_HEADER)
        .is_some_and(|value| state.secret.matches(value.as_bytes()));
    if !authenticated {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rate_admitted = match state.rate.lock() {
        Ok(mut rate) => rate.admit(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if !rate_admitted {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(permit) = state.admission.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    // Put the permit inside a separately scheduled task. If the deadline fires
    // while synchronous protobuf/SQLite work is using a runtime worker, the
    // permit remains held until that task is actually cancelled or completes.
    let mut task = tokio::spawn(async move {
        let _permit = permit;
        next.run(request).await
    });
    tokio::select! {
        result = &mut task => match result {
            Ok(response) => response,
            Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        },
        _ = tokio::time::sleep(PROCESSING_TIMEOUT) => {
            task.abort();
            StatusCode::REQUEST_TIMEOUT.into_response()
        }
    }
}

async fn receive_logs(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let request = match decode_http::<ExportLogsServiceRequest>(&headers, &body) {
        Ok(request) => request,
        Err(status) => return status,
    };
    if let Err(status) = validate_logs(&request) {
        return status;
    }
    persist(
        &state.database_path,
        parse_logs_with_identity(request, &state.receiver_identity),
    )
}

async fn receive_metrics(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let request = match decode_http::<ExportMetricsServiceRequest>(&headers, &body) {
        Ok(request) => request,
        Err(status) => return status,
    };
    if let Err(status) = validate_metrics(&request) {
        return status;
    }
    persist(
        &state.database_path,
        parse_metrics_with_identity(request, &state.receiver_identity),
    )
}

fn decode_http<T>(headers: &HeaderMap, body: &[u8]) -> Result<T, StatusCode>
where
    T: Message + serde::de::DeserializeOwned + Default,
{
    if body.len() > MAX_BODY_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    match content_type {
        Some(value) if value.eq_ignore_ascii_case("application/json") => {
            serde_json::from_slice(body).map_err(|_| StatusCode::BAD_REQUEST)
        }
        Some(value) if value.eq_ignore_ascii_case("application/x-protobuf") => {
            T::decode(body).map_err(|_| StatusCode::BAD_REQUEST)
        }
        _ => Err(StatusCode::UNSUPPORTED_MEDIA_TYPE),
    }
}

fn persist(database_path: &Path, events: Vec<ParsedEvent>) -> StatusCode {
    if events.len() > MAX_EVENTS {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    let store = match Store::open(database_path) {
        Ok(store) => store,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
    let batch = events
        .iter()
        .map(|event| OtelMetadata {
            id: &event.id,
            signal: &event.signal,
            event_name: event.event_name.as_deref(),
            occurred_at: event.occurred_at.as_deref(),
            thread_id: event.thread_id.as_deref(),
            turn_id: event.turn_id.as_deref(),
            model: event.model.as_deref(),
            provider: event.provider.as_deref(),
            status_code: event.status_code,
            duration_ms: event.duration_ms,
            response_bytes: event.response_bytes,
            endpoint: event.endpoint.as_deref(),
            success: event.success,
            input_tokens: event.input_tokens,
            cached_input_tokens: event.cached_input_tokens,
            cache_write_input_tokens: event.cache_write_input_tokens,
            output_tokens: event.output_tokens,
            reasoning_output_tokens: event.reasoning_output_tokens,
            total_tokens: event.total_tokens,
            attributes_json: &event.attributes_json,
        })
        .collect::<Vec<_>>();
    match store.insert_otel_batch(&batch) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Default)]
struct DecodeBudget {
    resources: usize,
    scopes: usize,
    events: usize,
    attributes: usize,
    data_points: usize,
}

impl DecodeBudget {
    fn add_resources(&mut self, amount: usize) -> Result<(), StatusCode> {
        self.resources = self.resources.saturating_add(amount);
        (self.resources <= MAX_RESOURCES)
            .then_some(())
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)
    }

    fn add_scopes(&mut self, amount: usize) -> Result<(), StatusCode> {
        self.scopes = self.scopes.saturating_add(amount);
        (self.scopes <= MAX_SCOPES)
            .then_some(())
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)
    }

    fn add_events(&mut self, amount: usize) -> Result<(), StatusCode> {
        self.events = self.events.saturating_add(amount);
        (self.events <= MAX_EVENTS)
            .then_some(())
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)
    }

    fn add_attributes(&mut self, values: &[KeyValue]) -> Result<(), StatusCode> {
        if values.len() > MAX_ATTRIBUTES_PER_ITEM {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        self.attributes = self.attributes.saturating_add(values.len());
        if self.attributes > MAX_ATTRIBUTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        for value in values {
            if value.key.len() > MAX_SCALAR_BYTES || value.key.chars().any(char::is_control) {
                return Err(StatusCode::UNPROCESSABLE_ENTITY);
            }
            if is_allowlisted(&value.key) {
                let scalar = scalar(value).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
                if value.key == "event.name" {
                    validate_event_name(&scalar)?;
                }
            }
        }
        Ok(())
    }

    fn add_data_points(&mut self, amount: usize) -> Result<(), StatusCode> {
        self.data_points = self.data_points.saturating_add(amount);
        (self.data_points <= MAX_DATA_POINTS)
            .then_some(())
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)
    }
}

fn validate_logs(request: &ExportLogsServiceRequest) -> Result<(), StatusCode> {
    let mut budget = DecodeBudget::default();
    budget.add_resources(request.resource_logs.len())?;
    for resource_logs in &request.resource_logs {
        validate_short_string(&resource_logs.schema_url, MAX_SCHEMA_STRING_BYTES)?;
        if let Some(resource) = &resource_logs.resource {
            budget.add_attributes(&resource.attributes)?;
        }
        budget.add_scopes(resource_logs.scope_logs.len())?;
        for scope in &resource_logs.scope_logs {
            validate_short_string(&scope.schema_url, MAX_SCHEMA_STRING_BYTES)?;
            if let Some(instrumentation) = &scope.scope {
                validate_short_string(&instrumentation.name, MAX_SCALAR_BYTES)?;
                validate_short_string(&instrumentation.version, MAX_SCALAR_BYTES)?;
                budget.add_attributes(&instrumentation.attributes)?;
            }
            budget.add_events(scope.log_records.len())?;
            for record in &scope.log_records {
                validate_event_name(&record.event_name)?;
                budget.add_attributes(&record.attributes)?;
            }
        }
    }
    Ok(())
}

fn validate_metrics(request: &ExportMetricsServiceRequest) -> Result<(), StatusCode> {
    let mut budget = DecodeBudget::default();
    budget.add_resources(request.resource_metrics.len())?;
    for resource_metrics in &request.resource_metrics {
        validate_short_string(&resource_metrics.schema_url, MAX_SCHEMA_STRING_BYTES)?;
        if let Some(resource) = &resource_metrics.resource {
            budget.add_attributes(&resource.attributes)?;
        }
        budget.add_scopes(resource_metrics.scope_metrics.len())?;
        for scope in &resource_metrics.scope_metrics {
            validate_short_string(&scope.schema_url, MAX_SCHEMA_STRING_BYTES)?;
            if let Some(instrumentation) = &scope.scope {
                validate_short_string(&instrumentation.name, MAX_SCALAR_BYTES)?;
                validate_short_string(&instrumentation.version, MAX_SCALAR_BYTES)?;
                budget.add_attributes(&instrumentation.attributes)?;
            }
            budget.add_events(scope.metrics.len())?;
            for metric in &scope.metrics {
                validate_event_name(&metric.name)?;
                validate_short_string(&metric.description, MAX_SCALAR_BYTES)?;
                validate_short_string(&metric.unit, MAX_SCALAR_BYTES)?;
                budget.add_attributes(&metric.metadata)?;
                validate_metric_data(metric, &mut budget)?;
            }
        }
    }
    Ok(())
}

fn validate_metric_data(metric: &Metric, budget: &mut DecodeBudget) -> Result<(), StatusCode> {
    macro_rules! validate_points {
        ($points:expr) => {{
            budget.add_data_points($points.len())?;
            for point in $points {
                budget.add_attributes(&point.attributes)?;
            }
        }};
    }
    match metric.data.as_ref() {
        Some(MetricData::Gauge(data)) => validate_points!(&data.data_points),
        Some(MetricData::Sum(data)) => validate_points!(&data.data_points),
        Some(MetricData::Histogram(data)) => validate_points!(&data.data_points),
        Some(MetricData::ExponentialHistogram(data)) => validate_points!(&data.data_points),
        Some(MetricData::Summary(data)) => validate_points!(&data.data_points),
        None => {}
    }
    Ok(())
}

fn validate_event_name(value: &str) -> Result<(), StatusCode> {
    validate_short_string(value, MAX_EVENT_NAME_BYTES)
}

fn validate_short_string(value: &str, limit: usize) -> Result<(), StatusCode> {
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

#[cfg(test)]
fn parse_logs(request: ExportLogsServiceRequest) -> Vec<ParsedEvent> {
    parse_logs_with_identity(request, "test-receiver")
}

fn parse_logs_with_identity(
    request: ExportLogsServiceRequest,
    receiver_identity: &str,
) -> Vec<ParsedEvent> {
    let now = Utc::now();
    let mut events = Vec::new();
    for resource_logs in request.resource_logs {
        let resource = attributes(
            resource_logs
                .resource
                .as_ref()
                .map(|resource| &resource.attributes),
        );
        parse_resource_logs(resource_logs, resource, receiver_identity, now, &mut events);
    }
    events
}

fn parse_resource_logs(
    resource_logs: ResourceLogs,
    resource: BTreeMap<String, String>,
    receiver_identity: &str,
    now: DateTime<Utc>,
    events: &mut Vec<ParsedEvent>,
) {
    for scope_logs in resource_logs.scope_logs {
        for record in scope_logs.log_records {
            // Deliberately never access `record.body`.
            let mut attrs = resource.clone();
            attrs.extend(attributes(Some(&record.attributes)));
            let occurred_at = timestamp(record.time_unix_nano, now)
                .or_else(|| timestamp(record.observed_time_unix_nano, now));
            let event_name =
                non_empty(record.event_name).or_else(|| attrs.get("event.name").cloned());
            events.push(event(
                receiver_identity,
                "logs",
                event_name,
                occurred_at,
                attrs,
            ));
        }
    }
}

fn parse_metrics_with_identity(
    request: ExportMetricsServiceRequest,
    receiver_identity: &str,
) -> Vec<ParsedEvent> {
    let now = Utc::now();
    let mut events = Vec::new();
    for resource_metrics in request.resource_metrics {
        let resource = attributes(
            resource_metrics
                .resource
                .as_ref()
                .map(|resource| &resource.attributes),
        );
        parse_resource_metrics(
            resource_metrics,
            resource,
            receiver_identity,
            now,
            &mut events,
        );
    }
    events
}

fn parse_resource_metrics(
    resource_metrics: ResourceMetrics,
    resource: BTreeMap<String, String>,
    receiver_identity: &str,
    now: DateTime<Utc>,
    events: &mut Vec<ParsedEvent>,
) {
    for scope_metrics in resource_metrics.scope_metrics {
        for metric in scope_metrics.metrics {
            let mut attrs = resource.clone();
            attrs.extend(attributes(Some(&metric.metadata)));
            let event_name = non_empty(metric.name);
            let occurred_at = metric_timestamp(metric.data.as_ref(), now);
            events.push(event(
                receiver_identity,
                "metrics",
                event_name,
                occurred_at,
                attrs,
            ));
        }
    }
}

fn metric_timestamp(data: Option<&MetricData>, now: DateTime<Utc>) -> Option<String> {
    let nanos = match data? {
        MetricData::Gauge(value) => value.data_points.first().map(|point| point.time_unix_nano),
        MetricData::Sum(value) => value.data_points.first().map(|point| point.time_unix_nano),
        MetricData::Histogram(value) => value.data_points.first().map(|point| point.time_unix_nano),
        MetricData::ExponentialHistogram(value) => {
            value.data_points.first().map(|point| point.time_unix_nano)
        }
        MetricData::Summary(value) => value.data_points.first().map(|point| point.time_unix_nano),
    }?;
    timestamp(nanos, now)
}

fn attributes(values: Option<&Vec<KeyValue>>) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for key_value in values.into_iter().flatten() {
        if is_allowlisted(&key_value.key) {
            if let Some(value) =
                scalar(key_value).and_then(|value| sanitize_attribute(&key_value.key, value))
            {
                result.entry(key_value.key.clone()).or_insert(value);
            }
        }
    }
    result
}

fn scalar(key_value: &KeyValue) -> Option<String> {
    let value = match key_value.value.as_ref()?.value.as_ref()? {
        AnyValue::StringValue(value) => Some(value.clone()),
        AnyValue::IntValue(value) => Some(value.to_string()),
        AnyValue::DoubleValue(value) if value.is_finite() => Some(value.to_string()),
        AnyValue::BoolValue(value) => Some(value.to_string()),
        _ => None,
    }?;
    (value.len() <= MAX_SCALAR_BYTES && !value.chars().any(char::is_control)).then_some(value)
}

fn sanitize_attribute(key: &str, value: String) -> Option<String> {
    match key {
        "event.name" => canonical_event_name(&value),
        "gen_ai.request.model" | "openai.request.model" | "codex.model" | "model" => {
            canonical_model(&value)
        }
        "gen_ai.system" | "provider" | "provider_name" => canonical_provider(&value),
        "codex.thread_id" | "thread.id" | "conversation.id" | "conversation_id" => {
            opaque_identifier("thread", &value)
        }
        "codex.turn_id" | "turn.id" | "turn_id" => opaque_identifier("turn", &value),
        "http.response.status_code" | "http.status_code" | "status" => {
            canonical_bounded_int(value, 999)
        }
        "gen_ai.client.operation.duration" | "duration_ms" => {
            canonical_bounded_int(value, MAX_DURATION_MS)
        }
        "http.response.body.size" | "response_bytes" => {
            canonical_bounded_int(value, MAX_RESPONSE_BYTES)
        }
        "gen_ai.usage.input_tokens"
        | "gen_ai.usage.cache_read.input_tokens"
        | "gen_ai.usage.cache_write.input_tokens"
        | "gen_ai.usage.cached_input_tokens"
        | "gen_ai.usage.cache_write_input_tokens"
        | "gen_ai.usage.output_tokens"
        | "gen_ai.usage.reasoning_output_tokens"
        | "gen_ai.usage.total_tokens"
        | "codex.usage.reasoning_output_tokens"
        | "codex.usage.total_tokens"
        | "input_token_count"
        | "cached_token_count"
        | "cache_write_token_count"
        | "output_token_count"
        | "reasoning_token_count"
        | "tool_token_count" => canonical_bounded_int(value, MAX_TOKEN_COUNT),
        "success" => match value.as_str() {
            "true" | "1" => Some("true".into()),
            "false" | "0" => Some("false".into()),
            _ => None,
        },
        "endpoint" => Some(value),
        _ => None,
    }
}

fn canonical_event_name(value: &str) -> Option<String> {
    match value {
        "" => None,
        "codex.api_request" => Some("codex.api_request".into()),
        "codex.sse_event" => Some("codex.sse_event".into()),
        _ => Some("other".into()),
    }
}

fn canonical_provider(value: &str) -> Option<String> {
    let canonical = match value.to_ascii_lowercase().as_str() {
        "openai" | "codex" => "openai",
        "azure" | "azure-openai" | "azure_openai" => "azure-openai",
        "aws" | "bedrock" | "aws-bedrock" | "aws_bedrock" => "aws-bedrock",
        "google" | "vertex" | "vertex-ai" | "google-vertex" => "google-vertex",
        "anthropic" => "anthropic",
        "" => return None,
        _ => "other",
    };
    Some(canonical.into())
}

fn canonical_model(value: &str) -> Option<String> {
    if let Some(model) = crate::pricing::known_model(value) {
        return Some(model.into());
    }
    opaque_identifier("model", value)
}

fn opaque_identifier(kind: &str, value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(b"codex-manager-otel-metadata-v1\0");
    digest.update(kind.as_bytes());
    digest.update(b"\0");
    digest.update(value.as_bytes());
    let encoded = hex::encode(digest.finalize());
    Some(format!("otel-{kind}-{}", &encoded[..24]))
}

fn canonical_bounded_int(value: String, maximum: i64) -> Option<String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| (0..=maximum).contains(value))
        .map(|value| value.to_string())
}

fn is_allowlisted(key: &str) -> bool {
    matches!(
        key,
        "event.name"
            | "gen_ai.request.model"
            | "openai.request.model"
            | "codex.model"
            | "model"
            | "gen_ai.system"
            | "provider"
            | "provider_name"
            | "codex.thread_id"
            | "thread.id"
            | "conversation.id"
            | "conversation_id"
            | "codex.turn_id"
            | "turn.id"
            | "turn_id"
            | "http.response.status_code"
            | "http.status_code"
            | "status"
            | "success"
            | "gen_ai.client.operation.duration"
            | "duration_ms"
            | "http.response.body.size"
            | "response_bytes"
            | "endpoint"
            | "gen_ai.usage.input_tokens"
            | "gen_ai.usage.cache_read.input_tokens"
            | "gen_ai.usage.cache_write.input_tokens"
            | "gen_ai.usage.cached_input_tokens"
            | "gen_ai.usage.cache_write_input_tokens"
            | "gen_ai.usage.output_tokens"
            | "gen_ai.usage.reasoning_output_tokens"
            | "gen_ai.usage.total_tokens"
            | "codex.usage.reasoning_output_tokens"
            | "codex.usage.total_tokens"
            | "input_token_count"
            | "cached_token_count"
            | "cache_write_token_count"
            | "output_token_count"
            | "reasoning_token_count"
            | "tool_token_count"
    )
}

fn event(
    receiver_identity: &str,
    signal: &str,
    event_name: Option<String>,
    occurred_at: Option<String>,
    mut attributes: BTreeMap<String, String>,
) -> ParsedEvent {
    let event_name = event_name.and_then(|value| canonical_event_name(&value));
    let endpoint = sanitize_endpoint(attributes.get("endpoint").map(String::as_str));
    if let Some(endpoint) = endpoint.as_ref() {
        attributes.insert("endpoint".into(), endpoint.clone());
    } else {
        attributes.remove("endpoint");
    }
    let model = value(
        &attributes,
        &[
            "gen_ai.request.model",
            "openai.request.model",
            "codex.model",
            "model",
        ],
    );
    let provider = value(&attributes, &["gen_ai.system", "provider", "provider_name"]);
    let thread_id = value(
        &attributes,
        &[
            "codex.thread_id",
            "thread.id",
            "conversation.id",
            "conversation_id",
        ],
    );
    let turn_id = value(&attributes, &["codex.turn_id", "turn.id", "turn_id"]);
    let status_code = bounded_int(
        &attributes,
        &["http.response.status_code", "http.status_code", "status"],
        999,
    );
    let success = boolean(&attributes, &["success"]);
    let duration_ms = bounded_int(
        &attributes,
        &["duration_ms", "gen_ai.client.operation.duration"],
        MAX_DURATION_MS,
    );
    let response_bytes = bounded_int(
        &attributes,
        &["http.response.body.size", "response_bytes"],
        MAX_RESPONSE_BYTES,
    );
    let input_tokens = token_int(
        &attributes,
        &["gen_ai.usage.input_tokens", "input_token_count"],
    );
    let cached_input_tokens = token_int(
        &attributes,
        &[
            "gen_ai.usage.cache_read.input_tokens",
            "gen_ai.usage.cached_input_tokens",
            "cached_token_count",
        ],
    );
    let cache_write_input_tokens = token_int(
        &attributes,
        &[
            "gen_ai.usage.cache_write.input_tokens",
            "gen_ai.usage.cache_write_input_tokens",
            "cache_write_token_count",
        ],
    );
    let output_tokens = token_int(
        &attributes,
        &["gen_ai.usage.output_tokens", "output_token_count"],
    );
    let reasoning_output_tokens = token_int(
        &attributes,
        &[
            "gen_ai.usage.reasoning_output_tokens",
            "codex.usage.reasoning_output_tokens",
            "reasoning_token_count",
        ],
    );
    let total_tokens = token_int(
        &attributes,
        &[
            "gen_ai.usage.total_tokens",
            "codex.usage.total_tokens",
            "tool_token_count",
        ],
    );
    let attributes_json = serde_json::to_string(&attributes).expect("BTreeMap serializes");
    let identity = format!(
        "{receiver_identity}|{signal}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        event_name, occurred_at, thread_id, turn_id, model, status_code, attributes_json
    );
    let mut digest = Sha256::new();
    digest.update(identity.as_bytes());
    ParsedEvent {
        id: hex::encode(digest.finalize()),
        signal: signal.into(),
        event_name,
        occurred_at,
        thread_id,
        turn_id,
        model,
        provider,
        status_code,
        duration_ms,
        response_bytes,
        endpoint,
        success,
        input_tokens,
        cached_input_tokens,
        cache_write_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
        attributes_json,
    }
}

fn value(attributes: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| attributes.get(*key).cloned())
}

fn token_int(attributes: &BTreeMap<String, String>, keys: &[&str]) -> Option<i64> {
    bounded_int(attributes, keys, MAX_TOKEN_COUNT)
}

fn bounded_int(attributes: &BTreeMap<String, String>, keys: &[&str], maximum: i64) -> Option<i64> {
    value(attributes, keys)
        .and_then(|value| value.parse().ok())
        .filter(|value| (0..=maximum).contains(value))
}

fn boolean(attributes: &BTreeMap<String, String>, keys: &[&str]) -> Option<bool> {
    match value(attributes, keys)?.as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Converts an untrusted URL into two fixed enums: a provider/origin class and
/// a known API route. No raw host, tenant subdomain, userinfo, query, fragment,
/// or unknown path survives.
fn sanitize_endpoint(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.len() > MAX_SCALAR_BYTES || value.chars().any(char::is_control) {
        return None;
    }
    if let Ok(url) = Url::parse(value) {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = url.host_str()?;
        let origin = classify_origin(host);
        return known_route(url.path())
            .map(|route| format!("{origin}:{route}"))
            .or_else(|| Some(origin.to_string()));
    }
    if value.starts_with('/') {
        return known_route(value).map(|route| format!("unknown:{route}"));
    }
    (value == "unknown").then(|| "unknown".to_string())
}

fn classify_origin(host: &str) -> &'static str {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost"
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    {
        "local"
    } else if domain_is(&host, "openai.com") {
        "openai"
    } else if domain_is(&host, "azure.com")
        || domain_is(&host, "azure-api.net")
        || domain_is(&host, "openai.azure.com")
    {
        "azure-openai"
    } else if domain_is(&host, "amazonaws.com") {
        "aws"
    } else if domain_is(&host, "googleapis.com") {
        "google"
    } else if domain_is(&host, "cloudflare.com") {
        "cloudflare"
    } else {
        "other"
    }
}

fn domain_is(host: &str, registrable: &str) -> bool {
    host == registrable || host.ends_with(&format!(".{registrable}"))
}

fn known_route(path: &str) -> Option<&'static str> {
    let normalized = path.trim_end_matches('/');
    match normalized {
        "/responses" | "/v1/responses" => Some("responses"),
        "/chat/completions" | "/v1/chat/completions" => Some("chat-completions"),
        "/embeddings" | "/v1/embeddings" => Some("embeddings"),
        "/models" | "/v1/models" => Some("models"),
        "/realtime" | "/v1/realtime" => Some("realtime"),
        "/v1/logs" => Some("otel-logs"),
        "/v1/metrics" => Some("otel-metrics"),
        _ => None,
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn timestamp(nanos: u64, now: DateTime<Utc>) -> Option<String> {
    if nanos == 0 {
        return None;
    }
    let seconds = i64::try_from(nanos / 1_000_000_000).ok()?;
    let subsec = (nanos % 1_000_000_000) as u32;
    let timestamp = DateTime::<Utc>::from_timestamp(seconds, subsec)?;
    let drift = timestamp.signed_duration_since(now);
    (drift >= -MAX_PAST_TIMESTAMP_DRIFT && drift <= MAX_FUTURE_TIMESTAMP_DRIFT)
        .then(|| timestamp.to_rfc3339())
}

fn ensure_tls_assets(data_dir: &Path) -> Result<TlsAssets, String> {
    let directory = data_dir.join("otel").join("tls");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建 OTLP TLS 目录：{error}"))?;
    set_private_mode(&directory, 0o700)?;
    let certificate_path = directory.join("localhost-cert.pem");
    let private_key_path = directory.join("localhost-key.pem");

    let existing_certificate = read_regular_bounded(&certificate_path)?;
    let existing_key = read_regular_bounded(&private_key_path)?;
    let existing_pair = match (existing_certificate, existing_key) {
        (Some(certificate), Some(key)) if validated_rustls_config(&certificate, &key).is_ok() => {
            Some((certificate, key))
        }
        _ => None,
    };
    let (certificate_pem, private_key_pem) = match existing_pair {
        Some(pair) => pair,
        None => generate_and_persist_tls_pair(&certificate_path, &private_key_path)?,
    };
    // Parsing proves the persisted certificate and key form one identity before
    // a listener is exposed.
    let _ = validated_rustls_config(&certificate_pem, &private_key_pem)?;
    Ok(TlsAssets {
        certificate_path,
        certificate_pem,
        private_key_pem,
    })
}

fn generate_and_persist_tls_pair(
    certificate_path: &Path,
    private_key_path: &Path,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    // A prior interrupted write or a mismatched pair is recoverable: both
    // app-owned names are atomically replaced and revalidated by the caller.
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|error| format!("无法生成 OTLP TLS 证书：{error}"))?;
    let certificate = cert.pem().into_bytes();
    let key = signing_key.serialize_pem().into_bytes();
    atomic_write_private(private_key_path, &key)?;
    atomic_write_private(certificate_path, &certificate)?;
    Ok((certificate, key))
}

fn derive_receiver_token(private_key_pem: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"codex-manager-otel-header-token-v1\0");
    digest.update(private_key_pem);
    hex::encode(digest.finalize())
}

fn rustls_config(certificate_pem: &[u8], private_key_pem: &[u8]) -> Result<RustlsConfig, String> {
    let certificates = rustls_pemfile::certs(&mut Cursor::new(certificate_pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("OTLP TLS 证书无法解析：{error}"))?;
    if certificates.len() != 1 {
        return Err("OTLP TLS 证书文件必须恰好包含一个证书".into());
    }
    let private_key = rustls_pemfile::private_key(&mut Cursor::new(private_key_pem))
        .map_err(|error| format!("OTLP TLS 私钥无法解析：{error}"))?
        .ok_or_else(|| "OTLP TLS 私钥文件为空".to_string())?;
    let provider = rustls::crypto::ring::default_provider();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("OTLP TLS 协议配置失败：{error}"))?
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|error| format!("OTLP TLS 证书与私钥不匹配：{error}"))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(RustlsConfig::from_config(Arc::new(config)))
}

fn validated_rustls_config(
    certificate_pem: &[u8],
    private_key_pem: &[u8],
) -> Result<RustlsConfig, String> {
    use rustls::{
        RootCertStore,
        client::{WebPkiServerVerifier, danger::ServerCertVerifier},
        pki_types::{ServerName, UnixTime},
    };

    let server_config = rustls_config(certificate_pem, private_key_pem)?;
    let certificates = rustls_pemfile::certs(&mut Cursor::new(certificate_pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("OTLP TLS 证书无法复核：{error}"))?;
    let certificate = certificates
        .first()
        .ok_or_else(|| "OTLP TLS 证书文件为空".to_string())?;
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate.clone())
        .map_err(|error| format!("OTLP TLS 证书无法作为本地信任根：{error}"))?;
    let provider = rustls::crypto::ring::default_provider();
    let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::new(provider))
        .build()
        .map_err(|error| format!("OTLP TLS localhost 校验器无法创建：{error}"))?;
    let localhost = ServerName::try_from("localhost")
        .map_err(|error| format!("OTLP TLS localhost 名称无效：{error}"))?;
    verifier
        .verify_server_cert(certificate, &[], &localhost, &[], UnixTime::now())
        .map_err(|error| format!("OTLP TLS 证书不匹配 localhost 或已失效：{error}"))?;
    Ok(server_config)
}

fn read_regular_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK prevents a malicious FIFO/device replacement from
        // stalling startup before descriptor metadata can reject it.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法安全打开 OTLP TLS 资产：{error}")),
    };
    // All security decisions and the bounded read are tied to this descriptor.
    // On Unix O_NOFOLLOW prevents the final component from being swapped to a
    // symlink between validation and use.
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法检查 OTLP TLS 资产句柄：{error}"))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_TLS_ASSET_BYTES {
        return Err(format!(
            "OTLP TLS 资产不是受支持的普通文件：{}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法收紧 OTLP 资产句柄权限：{error}"))?;
    }
    #[cfg(not(unix))]
    {
        // Windows support remains a later milestone, but never trust only a
        // pre-open pathname check: revalidate the current pathname after the
        // handle is acquired and refuse link-like/non-regular endpoints.
        let current = fs::symlink_metadata(path)
            .map_err(|error| format!("无法复核 OTLP TLS 资产路径：{error}"))?;
        if current.file_type().is_symlink()
            || !current.file_type().is_file()
            || current.len() != metadata.len()
        {
            return Err("OTLP TLS 资产在打开期间发生变化".into());
        }
    }
    let mut content = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_TLS_ASSET_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|error| format!("无法读取 OTLP TLS 资产：{error}"))?;
    if content.len() as u64 > MAX_TLS_ASSET_BYTES {
        return Err("OTLP TLS 资产超过大小限制".into());
    }
    Ok(Some(content))
}

fn atomic_write_private(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "OTLP TLS 资产缺少父目录".to_string())?;
    let temp_path = parent.join(format!(".otel-tls-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<(), String> {
        let mut file = options
            .open(&temp_path)
            .map_err(|error| format!("无法创建 OTLP TLS 临时文件：{error}"))?;
        file.write_all(content)
            .map_err(|error| format!("无法写入 OTLP TLS 临时文件：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("无法同步 OTLP TLS 临时文件：{error}"))?;
        fs::rename(&temp_path, path)
            .map_err(|error| format!("无法原子替换 OTLP TLS 资产：{error}"))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("无法同步 OTLP TLS 目录：{error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn set_private_mode(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("无法收紧 OTLP TLS 资产权限：{error}"))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use opentelemetry_proto::tonic::{
        common::v1::{AnyValue, KeyValue, any_value},
        logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
        metrics::v1::{ResourceMetrics, ScopeMetrics},
    };
    use tokio::io::AsyncReadExt;
    use tower::ServiceExt;

    fn attribute(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value.into())),
            }),
            key_strindex: 0,
        }
    }

    fn logs_request(records: Vec<LogRecord>) -> ExportLogsServiceRequest {
        ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }
    }

    fn test_state(database_path: PathBuf, token: &str) -> ReceiverState {
        ReceiverState {
            database_path,
            secret: Arc::new(ReceiverSecret::from_token(token).unwrap()),
            receiver_identity: "test-receiver".into(),
            admission: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            rate: Arc::new(Mutex::new(RateWindow::new())),
        }
    }

    fn http_request(
        route: &str,
        content_type: &str,
        body: Vec<u8>,
        token: Option<&str>,
    ) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri(route)
            .header(CONTENT_TYPE, content_type);
        if let Some(token) = token {
            builder = builder.header(TOKEN_HEADER, token);
        }
        builder.body(Body::from(body)).unwrap()
    }

    fn wait_until_port_released(port: u16) {
        for _ in 0..100 {
            if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                drop(listener);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("OTLP test listener did not release port {port}");
    }

    #[tokio::test]
    async fn idle_connection_times_out_and_releases_its_permit() {
        let permits = Arc::new(Semaphore::new(1));
        let permit = permits.clone().try_acquire_owned().unwrap();
        let (_client, server) = tokio::io::duplex(16);
        let mut guarded = ConnectionGuard::new(server, permit, Duration::from_millis(20));
        let mut byte = [0_u8; 1];

        let error = guarded.read_exact(&mut byte).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(guarded);
        assert_eq!(permits.available_permits(), 1);
    }

    #[test]
    fn tls_assets_are_private_and_reused() {
        let directory = tempfile::tempdir().unwrap();
        let first = ensure_tls_assets(directory.path()).unwrap();
        let second = ensure_tls_assets(directory.path()).unwrap();
        assert_eq!(first.certificate_pem, second.certificate_pem);
        assert_eq!(first.private_key_pem, second.private_key_pem);
        assert_eq!(
            derive_receiver_token(&first.private_key_pem),
            derive_receiver_token(&second.private_key_pem)
        );
        assert!(rustls_config(&first.certificate_pem, &first.private_key_pem).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_path = directory.path().join("otel/tls/localhost-key.pem");
            assert_eq!(
                fs::metadata(key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(directory.path().join("otel/tls"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn mismatched_tls_pair_is_atomically_recovered() {
        let first_directory = tempfile::tempdir().unwrap();
        let other_directory = tempfile::tempdir().unwrap();
        let first = ensure_tls_assets(first_directory.path()).unwrap();
        let other = ensure_tls_assets(other_directory.path()).unwrap();
        atomic_write_private(&first.certificate_path, &other.certificate_pem).unwrap();

        let recovered = ensure_tls_assets(first_directory.path()).unwrap();
        assert!(rustls_config(&recovered.certificate_pem, &recovered.private_key_pem).is_ok());
        assert_ne!(recovered.certificate_pem, other.certificate_pem);
        assert_ne!(recovered.private_key_pem, first.private_key_pem);
    }

    #[test]
    fn tls_certificate_without_localhost_identity_is_recovered() {
        let directory = tempfile::tempdir().unwrap();
        let first = ensure_tls_assets(directory.path()).unwrap();
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["example.invalid".into()]).unwrap();
        let foreign_certificate = cert.pem().into_bytes();
        atomic_write_private(&first.certificate_path, &foreign_certificate).unwrap();
        atomic_write_private(
            &directory.path().join("otel/tls/localhost-key.pem"),
            signing_key.serialize_pem().as_bytes(),
        )
        .unwrap();

        let recovered = ensure_tls_assets(directory.path()).unwrap();
        assert!(
            validated_rustls_config(&recovered.certificate_pem, &recovered.private_key_pem).is_ok()
        );
        assert_ne!(recovered.certificate_pem, foreign_certificate);
    }

    #[cfg(unix)]
    #[test]
    fn tls_asset_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let tls_directory = directory.path().join("otel/tls");
        fs::create_dir_all(&tls_directory).unwrap();
        let external = directory.path().join("external-cert.pem");
        fs::write(&external, b"not a certificate").unwrap();
        symlink(&external, tls_directory.join("localhost-cert.pem")).unwrap();

        let error = ensure_tls_assets(directory.path())
            .err()
            .expect("symlinked TLS asset must be rejected");
        assert!(error.contains("无法安全打开 OTLP TLS 资产"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receiver_restart_reuses_identity_credential_and_port() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let first = start(database.clone(), directory.path().to_path_buf()).unwrap();
        let first_token = first.token.clone();
        let first_port = first.port();
        let first_certificate = fs::read(first.certificate_path()).unwrap();
        first.stop();
        wait_until_port_released(first_port);

        let second = start(database, directory.path().to_path_buf()).unwrap();
        assert_eq!(second.token, first_token);
        assert_eq!(second.port(), first_port);
        assert_eq!(
            fs::read(second.certificate_path()).unwrap(),
            first_certificate
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("otel/receiver-port")).unwrap(),
            first_port.to_string()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(directory.path().join("otel/receiver-port"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        second.stop();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn persisted_port_conflict_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let first = start(database.clone(), directory.path().to_path_buf()).unwrap();
        let port = first.port();
        first.stop();
        wait_until_port_released(port);

        let blocker = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        let error = start(database, directory.path().to_path_buf())
            .err()
            .expect("persisted port conflict must fail closed");
        assert!(error.contains("无法绑定已保存的本机 OTLP 端口"), "{error}");
        assert_eq!(
            fs::read_to_string(directory.path().join("otel/receiver-port")).unwrap(),
            port.to_string()
        );
        drop(blocker);
    }

    #[tokio::test]
    async fn rejects_missing_wrong_token_and_origin_without_database_changes() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let token = "11".repeat(TOKEN_BYTES);
        let request = logs_request(vec![LogRecord::default()]);
        let body = serde_json::to_vec(&request).unwrap();

        let missing = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/logs",
                "application/json",
                body.clone(),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/metrics",
                "application/json",
                serde_json::to_vec(&ExportMetricsServiceRequest::default()).unwrap(),
                Some(&"22".repeat(TOKEN_BYTES)),
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let mut origin_request = http_request("/v1/logs", "application/json", body, Some(&token));
        origin_request
            .headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        let origin = receiver_router(test_state(database.clone(), &token))
            .oneshot(origin_request)
            .await
            .unwrap();
        assert_eq!(origin.status(), StatusCode::FORBIDDEN);

        let store = Store::open(&database).unwrap();
        let page = store
            .page_activity(&codex_storage::ActivityFilter::default())
            .unwrap();
        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn accepts_authenticated_json_and_protobuf() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let token = "33".repeat(TOKEN_BYTES);
        let log = LogRecord {
            event_name: "codex.api_request".into(),
            attributes: vec![attribute("model", "gpt-test")],
            ..Default::default()
        };
        let request = logs_request(vec![log]);
        let json = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/logs",
                "application/json; charset=utf-8",
                serde_json::to_vec(&request).unwrap(),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(json.status(), StatusCode::OK);

        let protobuf_log = logs_request(vec![LogRecord {
            event_name: "codex.api_request".into(),
            attributes: vec![attribute("model", "gpt-protobuf")],
            ..Default::default()
        }]);
        let protobuf = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/logs",
                "application/x-protobuf",
                protobuf_log.encode_to_vec(),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(protobuf.status(), StatusCode::OK);

        let metric_request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: None,
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![Metric {
                        name: "codex.api_request".into(),
                        metadata: vec![attribute("model", "gpt-test")],
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let metrics = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/metrics",
                "application/x-protobuf",
                metric_request.encode_to_vec(),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(metrics.status(), StatusCode::OK);

        let store = Store::open(&database).unwrap();
        let page = store
            .page_activity(&codex_storage::ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn rejects_whole_over_budget_batch() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let token = "44".repeat(TOKEN_BYTES);
        let request = logs_request(vec![LogRecord::default(); MAX_EVENTS + 1]);
        let response = receiver_router(test_state(database.clone(), &token))
            .oneshot(http_request(
                "/v1/logs",
                "application/x-protobuf",
                request.encode_to_vec(),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let store = Store::open(&database).unwrap();
        let page = store
            .page_activity(&codex_storage::ActivityFilter::default())
            .unwrap();
        assert_eq!(page.total, 0);
    }

    #[test]
    fn canonicalizes_credential_shaped_direct_event_name() {
        let request = logs_request(vec![LogRecord {
            event_name: "Bearer-REDACTED".into(),
            ..Default::default()
        }]);
        assert_eq!(validate_logs(&request), Ok(()));
        let event = parse_logs(request).remove(0);
        assert_eq!(event.event_name.as_deref(), Some("other"));
        assert!(!serde_json::to_string(&event).unwrap().contains("REDACTED"));
    }

    #[test]
    fn parses_metadata_without_body_or_endpoint_secrets() {
        let request = logs_request(vec![LogRecord {
            time_unix_nano: Utc::now().timestamp_nanos_opt().unwrap() as u64,
            body: Some(AnyValue {
                value: Some(any_value::Value::StringValue("message secret".into())),
            }),
            attributes: vec![
                attribute("event.name", "codex.api_request"),
                attribute("model", "gpt-5.6-sol"),
                attribute("conversation.id", "thread-1"),
                attribute(
                    "endpoint",
                    "https://tenant-secret:password@api.openai.com/v1/responses/api-key-secret?api_key=secret#fragment",
                ),
                attribute("success", "true"),
                attribute("provider", "Bearer-not-metadata"),
                attribute(
                    "input_token_count",
                    "999999999999999999999999999999999999999999",
                ),
                attribute("authorization", "Bearer never-store"),
            ],
            ..Default::default()
        }]);
        let events = parse_logs(request);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name.as_deref(), Some("codex.api_request"));
        assert_eq!(events[0].model.as_deref(), Some("gpt-5.6-sol"));
        assert!(
            events[0]
                .thread_id
                .as_deref()
                .is_some_and(|value| value.starts_with("otel-thread-"))
        );
        assert_ne!(events[0].thread_id.as_deref(), Some("thread-1"));
        assert_eq!(events[0].success, Some(true));
        assert_eq!(events[0].endpoint.as_deref(), Some("openai"));
        assert!(!events[0].attributes_json.contains("authorization"));
        assert!(!events[0].attributes_json.contains("message secret"));
        assert!(!events[0].attributes_json.contains("api_key"));
        assert!(!events[0].attributes_json.contains("password"));
        assert!(!events[0].attributes_json.contains("999999999999"));
        assert!(!events[0].attributes_json.contains("Bearer"));
        assert_eq!(events[0].provider.as_deref(), Some("other"));

        assert_eq!(
            sanitize_endpoint(Some(
                "https://tenant-very-secret.proxy.example/v1/responses?token=secret"
            )),
            Some("other:responses".into())
        );
        assert_eq!(
            sanitize_endpoint(Some("https://api.openai.com/v1/responses")),
            Some("openai:responses".into())
        );
    }

    #[test]
    fn arbitrary_free_text_metadata_never_persists_verbatim() {
        let canary = "opaque-canary-value-47a91b";
        let request = logs_request(vec![LogRecord {
            event_name: "codex.api_request".into(),
            attributes: vec![
                attribute("model", canary),
                attribute("provider", canary),
                attribute("conversation.id", canary),
                attribute("turn.id", canary),
            ],
            ..Default::default()
        }]);

        assert_eq!(validate_logs(&request), Ok(()));
        let event = parse_logs(request).remove(0);
        let persisted_projection = serde_json::to_string(&event).unwrap();
        assert!(!persisted_projection.contains(canary));
        assert_eq!(event.event_name.as_deref(), Some("codex.api_request"));
        assert_eq!(event.provider.as_deref(), Some("other"));
        assert!(
            event
                .model
                .as_deref()
                .is_some_and(|value| value.starts_with("otel-model-"))
        );
        assert!(
            event
                .thread_id
                .as_deref()
                .is_some_and(|value| value.starts_with("otel-thread-"))
        );
        assert!(
            event
                .turn_id
                .as_deref()
                .is_some_and(|value| value.starts_with("otel-turn-"))
        );

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        assert_eq!(persist(&database, vec![event]), StatusCode::OK);
        let page = Store::open(&database)
            .unwrap()
            .page_activity(&codex_storage::ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert!(!serde_json::to_string(&page.items).unwrap().contains(canary));
    }

    #[test]
    fn rejects_future_timestamp_and_namespaces_event_identity() {
        let future = (Utc::now() + chrono::Duration::hours(1))
            .timestamp_nanos_opt()
            .unwrap() as u64;
        let request = logs_request(vec![LogRecord {
            time_unix_nano: future,
            event_name: "codex.request".into(),
            ..Default::default()
        }]);
        let first = parse_logs_with_identity(request.clone(), "receiver-a");
        let same = parse_logs_with_identity(request.clone(), "receiver-a");
        let other = parse_logs_with_identity(request, "receiver-b");
        assert_eq!(first[0].occurred_at, None);
        assert_eq!(first[0].id, same[0].id);
        assert_ne!(first[0].id, other[0].id);
    }

    #[test]
    fn config_snippet_is_the_only_explicit_credential_surface() {
        let directory = tempfile::tempdir().unwrap();
        let assets = ensure_tls_assets(directory.path()).unwrap();
        let token = "55".repeat(TOKEN_BYTES);
        let server = ServerHandle {
            port: 4318,
            endpoint: "https://localhost:4318".into(),
            certificate_path: assets.certificate_path,
            token: token.clone(),
            server: AxumServerHandle::new(),
        };
        assert_eq!(server.port(), 4318);
        assert_eq!(server.endpoint(), "https://localhost:4318");
        assert!(server.certificate_path().ends_with("localhost-cert.pem"));
        let snippet = server.config_snippet();
        assert!(snippet.contains(&token));
        assert!(snippet.contains(TOKEN_HEADER));
        assert!(snippet.contains("ca-certificate"));
        assert!(toml::from_str::<toml::Value>(&snippet).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tls_endpoint_accepts_only_the_pinned_localhost_identity() {
        use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
        use std::net::TcpStream;

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("otel.db");
        let server = start(database, directory.path().to_path_buf()).unwrap();
        let port = server.port();
        let token = server.token.clone();
        let certificate = fs::read(server.certificate_path()).unwrap();
        let request = logs_request(vec![LogRecord {
            event_name: "codex.tls_test".into(),
            ..Default::default()
        }])
        .encode_to_vec();

        let response = tokio::task::spawn_blocking(move || {
            let certificates = rustls_pemfile::certs(&mut Cursor::new(certificate))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let mut roots = RootCertStore::empty();
            roots.add(certificates[0].clone()).unwrap();
            let provider = rustls::crypto::ring::default_provider();
            let config = ClientConfig::builder_with_provider(Arc::new(provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let name = "localhost".try_into().unwrap();
            let connection = ClientConnection::new(Arc::new(config), name).unwrap();
            let mut tcp = None;
            for _ in 0..40 {
                match TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
                    Ok(stream) => {
                        tcp = Some(stream);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let mut stream = StreamOwned::new(connection, tcp.expect("TLS listener starts"));
            stream
                .sock
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                stream,
                "POST /v1/logs HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-protobuf\r\n{TOKEN_HEADER}: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                request.len()
            )
            .unwrap();
            stream.write_all(&request).unwrap();
            stream.flush().unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
        .await
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        server.stop();
    }
}
