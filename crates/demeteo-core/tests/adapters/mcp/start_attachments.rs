// Router-level tests for `start_feature` attachments. `super` =
// `adapters::mcp::mcp_handler`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{ClientId, GrantId, ProjectId, WorkflowId};
use crate::domain::models::{EffortLevel, Feature, Project, Workflow};
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::ports::step_executor::{FeatureLaunch, StepExecutor};
use crate::state::AppContext;

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-image";
const TOKEN: &str = "token-spend";

/// Captures the [`FeatureLaunch`] `start_feature` builds and answers with a fixed
/// `Feature`; every other call is a test failure.
struct SpyExecutor {
    captured: Mutex<Option<FeatureLaunch>>,
    calls: AtomicUsize,
}

impl SpyExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(None),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl StepExecutor for SpyExecutor {
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let feature = Feature {
            id: crate::domain::ids::FeatureId::from("f-1".to_string()),
            project_id: ProjectId::from(launch.project_id.clone()),
            workflow_id: None,
            workflow_version_id: None,
            title: launch.title.clone(),
            description: launch.description.clone(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: None,
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        };
        *self.captured.lock().expect("lock is not poisoned") = Some(launch);
        Ok(feature)
    }

    async fn feature_pause(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resume(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_cancel(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_get(&self, _: &str) -> Result<crate::domain::models::StepExecution, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_retry(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_set_assignment(
        &self,
        _: &str,
        _: crate::domain::step_assignment::StepAssignment,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn replay_from_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_list_for_run(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::models::StepExecution>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_drift(
        &self,
        _: &str,
        _: bool,
    ) -> Result<crate::domain::models::FeatureDrift, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_reconcile(
        &self,
        _: &str,
        _: crate::domain::upstream_feature::DivergenceReconcile,
    ) -> Result<Option<crate::ports::sync_session::SyncSessionView>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_divergence(
        &self,
        _: &str,
    ) -> Result<Option<crate::domain::models::FeatureDivergence>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _: &str,
        _: &crate::domain::sync_resolver::SyncResolverChoice,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_continue_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync_resolver(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncResolverView, String> {
        panic!("unexpected StepExecutor call")
    }
}

impl SpyExecutor {
    fn launch(&self) -> Option<FeatureLaunch> {
        self.captured.lock().expect("lock is not poisoned").clone()
    }

    fn feature_start_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// The spy goes onto the context before `router` clones it, so the route under
/// test reaches the spy and not the real executor.
async fn spawn_router(tag: &str) -> (SocketAddr, Arc<SpyExecutor>, AppContext) {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-mcp-start-attachments-{tag}"));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let spy = SpyExecutor::new();
    ctx.executor = spy.clone();
    ctx.projects
        .add(Project {
            id: ProjectId::from("p-1".to_string()),
            name: "project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("seed project");
    ctx.workflows
        .create(Workflow {
            id: WorkflowId::from("wf-1".to_string()),
            name: "wf".to_string(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        })
        .expect("seed workflow");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let resource = record_canonical_uri(addr);

    ctx.oauth_clients
        .register_client(OAuthClient {
            id: ClientId::new("client-1"),
            client_name: "test-client".to_string(),
            redirect_uris: vec![],
            created_at: crate::paths::now_ms(),
        })
        .expect("register test client");
    ctx.oauth_grants
        .insert_grant(
            GrantRecord {
                id: GrantId::new("grant-1"),
                client_id: ClientId::new("client-1"),
                scopes: vec![Scope::Spend],
                resource,
                issued_at: crate::paths::now_ms(),
                expires_at: crate::paths::now_ms() + 3_600_000,
                revoked_at: None,
            },
            &hash_token(TOKEN),
        )
        .expect("insert test grant");

    let app = router(ctx.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, spy, ctx)
}

fn base_arguments() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "project_id": "p-1",
        "workflow_id": "wf-1",
        "title": "the title",
        "description": "the description",
    }) else {
        unreachable!("a json! object literal is an object")
    };
    map
}

async fn call_start_feature(addr: SocketAddr, arguments: Value) -> Value {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "start_feature", "arguments": arguments },
        }))
        .send()
        .await
        .expect("request /mcp");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.expect("the response is JSON")
}

fn with_attachments(attachments: Value) -> Value {
    let mut arguments = base_arguments();
    arguments.insert("attachments".to_string(), attachments);
    Value::Object(arguments)
}

fn write_png(tag: &str, name: &str) -> PathBuf {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-mcp-start-attachments-{tag}"));
    let path = dir.join(name);
    std::fs::write(&path, PNG_BYTES).expect("write the PNG fixture");
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

fn assert_succeeded(response: &Value) {
    assert!(response.get("error").is_none(), "rpc error: {response}");
    assert_eq!(response["result"]["isError"], json!(false), "{response}");
}

fn assert_one_png(launch: &FeatureLaunch, filename: &str) {
    let [staged] = launch.staged_attachments.as_slice() else {
        panic!("expected one staged attachment, got {launch:?}");
    };
    assert_eq!(staged.bytes.as_deref(), Some(PNG_BYTES));
    assert_eq!(staged.source_path, "");
    assert_eq!(staged.mime.as_deref(), Some("image/png"));
    assert_eq!(staged.source_filename.as_deref(), Some(filename));
}

#[tokio::test]
async fn a_png_attached_by_path_reaches_the_executor_as_one_validated_input() {
    let (addr, spy, _ctx) = spawn_router("by-path").await;
    let png = write_png("by-path-src", "capture.png");

    let response =
        call_start_feature(addr, with_attachments(json!([{ "path": path_str(&png) }]))).await;

    assert_succeeded(&response);
    assert_one_png(
        &spy.launch().expect("feature_start was called"),
        "capture.png",
    );
}

#[tokio::test]
async fn a_png_attached_inline_reaches_the_executor_as_one_validated_input() {
    let (addr, spy, _ctx) = spawn_router("inline").await;
    let inline = json!([{
        "content_base64": STANDARD.encode(PNG_BYTES),
        "filename": "shot.png",
    }]);

    let response = call_start_feature(addr, with_attachments(inline)).await;

    assert_succeeded(&response);
    assert_one_png(&spy.launch().expect("feature_start was called"), "shot.png");
}

#[tokio::test]
async fn the_same_bytes_attached_twice_in_one_call_yield_one_entry() {
    let (addr, spy, _ctx) = spawn_router("dedup").await;
    let png = write_png("dedup-src", "capture.png");
    let both = json!([
        { "path": path_str(&png) },
        { "content_base64": STANDARD.encode(PNG_BYTES), "filename": "shot.png" },
    ]);

    let response = call_start_feature(addr, with_attachments(both)).await;

    assert_succeeded(&response);
    let launch = spy.launch().expect("feature_start was called");
    assert_eq!(launch.staged_attachments.len(), 1);
    assert_eq!(
        launch.staged_attachments[0].bytes.as_deref(),
        Some(PNG_BYTES)
    );
}

async fn assert_unattached_launch(arguments: Value, tag: &str) {
    let (addr, spy, _ctx) = spawn_router(tag).await;

    let response = call_start_feature(addr, arguments).await;

    assert_succeeded(&response);
    let launch = spy.launch().expect("feature_start was called");
    assert!(launch.staged_attachments.is_empty());
    assert_eq!(launch.project_id, "p-1");
    assert_eq!(launch.workflow_id, "wf-1");
    assert_eq!(launch.title, "the title");
    assert_eq!(launch.description, "the description");
    assert_eq!(launch.feature_id, None);
    assert_eq!(launch.agent_kind, None);
    assert_eq!(launch.origin, FeatureOrigin::DefaultBranch);
}

#[tokio::test]
async fn omitted_attachments_leave_the_launch_unchanged() {
    assert_unattached_launch(Value::Object(base_arguments()), "omitted").await;
}

#[tokio::test]
async fn an_empty_attachments_array_leaves_the_launch_unchanged() {
    assert_unattached_launch(with_attachments(json!([])), "empty").await;
}

#[tokio::test]
async fn a_wrongly_typed_attachments_value_is_invalid_params() {
    let (addr, spy, _ctx) = spawn_router("wrong-type").await;

    let response = call_start_feature(addr, with_attachments(json!("x"))).await;

    assert_eq!(response["error"]["code"], json!(-32602), "{response}");
    assert!(spy.launch().is_none());
}

fn write_file(tag: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-mcp-start-attachments-{tag}"));
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write the fixture");
    path
}

fn write_sparse(tag: &str, name: &str, header: &[u8], len: u64) -> PathBuf {
    let path = write_file(tag, name, header);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("reopen the fixture")
        .set_len(len)
        .expect("extend the fixture to a sparse file");
    path
}

fn inline_png(filename: &str, tail: &str) -> Value {
    let mut bytes = PNG_BYTES.to_vec();
    bytes.extend_from_slice(tail.as_bytes());
    json!({ "content_base64": STANDARD.encode(bytes), "filename": filename })
}

fn refusal_text(response: &Value) -> &str {
    assert!(response.get("error").is_none(), "rpc error: {response}");
    assert_eq!(response["result"]["isError"], json!(true), "{response}");
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a refusal carries text: {response}"))
}

async fn assert_started_nothing(spy: &SpyExecutor, ctx: &AppContext) {
    assert_eq!(spy.feature_start_calls(), 0);
    let rows = ctx
        .features
        .get_all_for_project(&ProjectId::from("p-1".to_string()))
        .expect("list the project's features");
    assert!(rows.is_empty(), "a refusal left rows behind: {rows:?}");
}

/// `index` is the item the message must name; `None` for count and batch-size
/// refusals, which carry no index.
async fn assert_refused(tag: &str, attachments: Value, index: Option<usize>, cause: &str) {
    let (addr, spy, ctx) = spawn_router(tag).await;

    let response = call_start_feature(addr, with_attachments(attachments)).await;

    let text = refusal_text(&response);
    match index {
        Some(i) => assert!(
            text.starts_with(&format!("attachments[{i}]: ")),
            "wrong prefix: {text}"
        ),
        None => assert!(
            !text.starts_with("attachments["),
            "unexpected index: {text}"
        ),
    }
    assert!(text.contains(cause), "`{cause}` not in: {text}");
    assert_started_nothing(&spy, &ctx).await;
}

#[tokio::test]
async fn a_missing_file_is_refused() {
    let dir = crate::support::test_dir::scratch("demeteo-mcp-start-attachments-missing-src");
    let gone = dir.join("gone.png");
    assert_refused(
        "missing",
        json!([{ "path": path_str(&gone) }]),
        Some(0),
        "could not read the file at path",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_txt_symlink_to_an_extensionless_file_is_refused() {
    let target = write_file(
        "symlink-type",
        "credentials",
        b"aws_secret_access_key = x\n",
    );
    let link = target.with_file_name("notes.txt");
    std::os::unix::fs::symlink(&target, &link).expect("link a .txt name to the file");
    assert_refused(
        "symlink-type",
        json!([{ "path": path_str(&link) }]),
        Some(0),
        "could not read the file at path",
    )
    .await;
}

#[tokio::test]
async fn a_directory_is_refused() {
    let dir =
        crate::support::test_dir::scratch("demeteo-mcp-start-attachments-dir-src").join("x.png");
    std::fs::create_dir_all(&dir).expect("make a directory named like an image");
    assert_refused(
        "dir",
        json!([{ "path": path_str(&dir) }]),
        Some(0),
        "could not read the file at path",
    )
    .await;
}

#[tokio::test]
async fn a_relative_path_is_refused() {
    assert_refused(
        "relative",
        json!([{ "path": PathBuf::from("shots").join("capture.png").to_str() }]),
        Some(0),
        "path must be absolute",
    )
    .await;
}

#[tokio::test]
async fn an_empty_file_is_refused() {
    let empty = write_file("empty-file-src", "empty.png", b"");
    assert_refused(
        "empty-file",
        json!([{ "path": path_str(&empty) }]),
        Some(0),
        "attachment bytes are empty",
    )
    .await;
}

#[tokio::test]
async fn an_empty_content_base64_is_refused() {
    assert_refused(
        "empty-inline",
        json!([{ "content_base64": "", "filename": "shot.png" }]),
        Some(0),
        "exactly one of path or content_base64",
    )
    .await;
}

#[tokio::test]
async fn a_file_over_the_size_limit_is_refused() {
    let big = write_sparse(
        "oversize-src",
        "big.png",
        PNG_BYTES,
        crate::application::attachments::MAX_ATTACHMENT_BYTES + 1,
    );
    assert_refused(
        "oversize",
        json!([{ "path": path_str(&big) }]),
        Some(0),
        "attachment too large",
    )
    .await;
}

#[tokio::test]
async fn an_executable_is_refused() {
    let exe = write_file("exe-src", "setup.exe", b"MZ\x90\x00");
    assert_refused(
        "exe",
        json!([{ "path": path_str(&exe) }]),
        Some(0),
        "unsupported attachment type",
    )
    .await;
}

#[tokio::test]
async fn a_zip_mime_is_refused() {
    let png = write_png("zip-src", "capture.png");
    assert_refused(
        "zip",
        json!([{ "path": path_str(&png), "mime": "application/zip" }]),
        Some(0),
        "does not match the file's type image/png",
    )
    .await;
}

#[tokio::test]
async fn an_octet_stream_mime_cannot_change_what_the_file_name_says() {
    let png = write_png("octet-src", "capture.png");
    assert_refused(
        "octet",
        json!([{ "path": path_str(&png), "mime": "application/octet-stream" }]),
        Some(0),
        "does not match the file's type image/png",
    )
    .await;
}

#[tokio::test]
async fn a_text_file_declared_as_an_image_is_refused_as_a_mismatch() {
    let text = write_file("text-as-png-src", "notes.txt", b"just words");
    assert_refused(
        "text-as-png",
        json!([{ "path": path_str(&text), "mime": "image/png" }]),
        Some(0),
        "does not match the file's type text/plain",
    )
    .await;
}

#[tokio::test]
async fn a_png_named_file_with_text_inside_is_refused_on_its_content() {
    let fake = write_file("fake-png-src", "capture.png", b"just words");
    assert_refused(
        "fake-png",
        json!([{ "path": path_str(&fake) }]),
        Some(0),
        "content does not look like image/png",
    )
    .await;
}

#[tokio::test]
async fn png_bytes_declared_as_a_pdf_are_refused() {
    let png = write_png("png-as-pdf-src", "capture.png");
    assert_refused(
        "png-as-pdf",
        json!([{ "path": path_str(&png), "mime": "application/pdf" }]),
        Some(0),
        "does not match the file's type image/png",
    )
    .await;
}

#[tokio::test]
async fn invalid_base64_is_refused() {
    assert_refused(
        "bad-base64",
        json!([{ "content_base64": "!!not base64!!", "filename": "shot.png" }]),
        Some(0),
        "content_base64 is not valid base64",
    )
    .await;
}

#[tokio::test]
async fn an_item_with_neither_path_nor_content_is_refused() {
    assert_refused(
        "neither",
        json!([{ "filename": "shot.png" }]),
        Some(0),
        "exactly one of path or content_base64",
    )
    .await;
}

#[tokio::test]
async fn an_item_with_both_path_and_content_is_refused() {
    let png = write_png("both-src", "capture.png");
    assert_refused(
        "both",
        json!([{
            "path": path_str(&png),
            "content_base64": STANDARD.encode(PNG_BYTES),
            "filename": "shot.png",
        }]),
        Some(0),
        "exactly one of path or content_base64",
    )
    .await;
}

#[tokio::test]
async fn an_inline_item_without_a_filename_is_refused() {
    assert_refused(
        "no-filename",
        json!([{ "content_base64": STANDARD.encode(PNG_BYTES) }]),
        Some(0),
        "filename is required",
    )
    .await;
}

#[tokio::test]
async fn eleven_distinct_files_are_refused() {
    let items: Vec<Value> = (0..11)
        .map(|i| inline_png(&format!("shot-{i}.png"), &i.to_string()))
        .collect();
    assert_refused(
        "eleven",
        Value::Array(items),
        None,
        "feature would have 11 attachments (max 10)",
    )
    .await;
}

#[tokio::test]
async fn a_batch_over_the_aggregate_cap_is_refused() {
    use crate::application::attachments::{MAX_AGENT_ATTACHMENT_BATCH_BYTES, MAX_ATTACHMENT_BYTES};
    let files = MAX_AGENT_ATTACHMENT_BATCH_BYTES / MAX_ATTACHMENT_BYTES + 1;
    let items: Vec<Value> = (0..files)
        .map(|i| {
            let header = format!("%PDF-1.{i}");
            let path = write_sparse(
                "aggregate-src",
                &format!("big-{i}.pdf"),
                header.as_bytes(),
                MAX_ATTACHMENT_BYTES,
            );
            json!({ "path": path_str(&path) })
        })
        .collect();
    assert_refused(
        "aggregate",
        Value::Array(items),
        None,
        "attachments are too large together",
    )
    .await;
}

#[tokio::test]
async fn one_valid_and_one_invalid_item_start_nothing() {
    let png = write_png("mixed-src", "capture.png");
    let dir = crate::support::test_dir::scratch("demeteo-mcp-start-attachments-mixed-gone");
    let gone = dir.join("gone.png");
    assert_refused(
        "mixed",
        json!([{ "path": path_str(&png) }, { "path": path_str(&gone) }]),
        Some(1),
        "could not read the file at path",
    )
    .await;
}

#[tokio::test]
async fn eleven_items_that_dedup_to_ten_are_accepted() {
    let (addr, spy, _ctx) = spawn_router("eleven-dedup").await;
    let mut items: Vec<Value> = (0..10)
        .map(|i| inline_png(&format!("shot-{i}.png"), &i.to_string()))
        .collect();
    items.push(inline_png("again.png", "0"));

    let response = call_start_feature(addr, with_attachments(Value::Array(items))).await;

    assert_succeeded(&response);
    let launch = spy.launch().expect("feature_start was called");
    assert_eq!(launch.staged_attachments.len(), 10);
    assert_eq!(spy.feature_start_calls(), 1);
}

fn start_feature_descriptor() -> Value {
    let catalog = super::tool_catalog();
    catalog
        .as_array()
        .and_then(|tools| tools.iter().find(|t| t["name"] == "start_feature"))
        .cloned()
        .expect("start_feature is in the catalog")
}

fn resolve_ref<'a>(schema: &'a Value, node: &'a Value) -> &'a Value {
    let Some(reference) = node.get("$ref").and_then(Value::as_str) else {
        return node;
    };
    let pointer = reference.trim_start_matches('#');
    schema
        .pointer(pointer)
        .expect("$ref resolves inside the input schema")
}

#[test]
fn the_start_feature_schema_documents_attachments_as_an_optional_array_of_objects() {
    let descriptor = start_feature_descriptor();
    let schema = &descriptor["inputSchema"];

    let attachments = resolve_ref(schema, &schema["properties"]["attachments"]);
    assert!(
        type_includes(&attachments["type"], "array"),
        "attachments is not an array: {attachments}"
    );
    let item = resolve_ref(schema, &attachments["items"]);
    assert!(
        type_includes(&item["type"], "object"),
        "attachment items are not objects: {item}"
    );
    for field in ["path", "content_base64", "mime", "filename"] {
        let description = item["properties"][field]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(!description.trim().is_empty(), "{field} has no description");
    }

    let required = schema["required"].as_array().cloned().unwrap_or_default();
    assert!(!required.iter().any(|name| name == "attachments"));
}

fn type_includes(declared: &Value, wanted: &str) -> bool {
    match declared {
        Value::String(single) => single == wanted,
        Value::Array(many) => many.iter().any(|t| t == wanted),
        _ => false,
    }
}

#[test]
fn the_start_feature_description_states_what_is_accepted() {
    let descriptor = start_feature_descriptor();
    let description = descriptor["description"].as_str().unwrap_or_default();

    for needle in [
        "png",
        "jpg",
        "gif",
        "webp",
        "tiff",
        "pdf",
        "txt",
        "md",
        "json",
        "100 MiB",
        "2 MiB",
        "Demeteo host",
        "at most 10",
        "exactly one",
        "filename",
        "mime",
        "data directory",
        "at most 20 items",
        "An invalid attachment refuses the whole call and starts nothing",
    ] {
        assert!(
            description.contains(needle),
            "description does not mention {needle:?}: {description}"
        );
    }
    assert!(
        !description.contains("by file extension."),
        "the type is not always taken from the extension: {description}"
    );
}

#[tokio::test]
async fn a_filename_cannot_launder_a_file_with_no_accepted_extension() {
    let creds = write_file("launder-src", "credentials", b"aws_secret_access_key = x");
    assert_refused(
        "launder",
        json!([{ "path": path_str(&creds), "filename": "x.txt" }]),
        Some(0),
        "unsupported attachment type",
    )
    .await;
}

#[tokio::test]
async fn a_path_inside_the_data_directory_is_refused_and_starts_nothing() {
    let (addr, spy, ctx) = spawn_router("data-dir").await;
    let staged = ctx.app_data_dir.join("attachments").join("f-0");
    std::fs::create_dir_all(&staged).expect("make the store directory");
    let secret = staged.join("notes.txt");
    std::fs::write(&secret, b"another feature's bytes").expect("write the fixture");

    let response = call_start_feature(
        addr,
        with_attachments(json!([{ "path": path_str(&secret) }])),
    )
    .await;

    let text = refusal_text(&response);
    assert!(text.contains("Demeteo's own data directory"), "{text}");
    assert_started_nothing(&spy, &ctx).await;
}

#[tokio::test]
async fn a_mockup_inside_the_workspace_is_accepted_even_when_it_sits_in_the_data_directory() {
    let (addr, spy, ctx) = spawn_router("workspace-mockup").await;
    assert_eq!(ctx.workspace_dir, ctx.app_data_dir, "the default layout");
    let repo = ctx.workspace_dir.join("repos").join("app");
    std::fs::create_dir_all(&repo).expect("make a project repo directory");
    let mockup = repo.join("mock.png");
    std::fs::write(&mockup, PNG_BYTES).expect("write the fixture");

    let response = call_start_feature(
        addr,
        with_attachments(json!([{ "path": path_str(&mockup) }])),
    )
    .await;

    assert_succeeded(&response);
    let launch = spy.launch().expect("feature_start was called");
    assert_one_png(&launch, "mock.png");
}
