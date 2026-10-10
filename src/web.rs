//! Local web UI. Binds to 127.0.0.1 only, so nothing on your network can reach it.

use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Request},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use papervault::tools::{self, Opts, Upload};
use serde_json::json;

const INDEX: &str = include_str!("../web/index.html");

/// Reject requests whose Host header is not our own address. This blocks DNS-rebinding attacks,
/// where a malicious web page tries to reach the local server through a hostname it controls.
async fn check_host(headers: HeaderMap, request: Request, next: Next) -> Response {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let allowed = host.starts_with("127.0.0.1:") || host.starts_with("localhost:") || host == "127.0.0.1" || host == "localhost";
    if !allowed {
        return (StatusCode::FORBIDDEN, "forbidden host").into_response();
    }
    next.run(request).await
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn list_tools() -> Json<serde_json::Value> {
    let tools: Vec<_> = tools::TOOLS
        .iter()
        .map(|t| json!({ "id": t.id, "summary": t.summary, "input": t.input, "options": t.options }))
        .collect();
    Json(json!({ "tools": tools }))
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn run_tool(Path(tool): Path<String>, mut multipart: Multipart) -> Response {
    let mut files = Vec::new();
    let mut opts = Opts::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => return error(StatusCode::BAD_REQUEST, format!("bad upload: {e}")),
        };
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let file_name = field.file_name().unwrap_or("file").to_string();
            match field.bytes().await {
                Ok(data) => files.push(Upload { name: file_name, data: data.to_vec() }),
                Err(e) => return error(StatusCode::BAD_REQUEST, format!("could not read upload: {e}")),
            }
        } else if let Ok(value) = field.text().await {
            opts.insert(name, value);
        }
    }

    // CPU-heavy work goes to a blocking thread so the server stays responsive.
    let result = tokio::task::spawn_blocking(move || -> Result<(String, &'static str, Vec<u8>)> {
        let outputs = tools::run(&tool, &files, &opts)?;
        match outputs.as_slice() {
            [single] => Ok((single.name.clone(), mime_for(&single.name), single.data.clone())),
            _ => Ok(("papervault-results.zip".to_string(), "application/zip", tools::zip(&outputs)?)),
        }
    })
    .await;

    match result {
        Ok(Ok((name, mime, data))) => (
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            ],
            Body::from(data),
        )
            .into_response(),
        Ok(Err(e)) => error(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("internal error: {e}")),
    }
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Start the server and (optionally) open the UI in the default browser.
pub fn serve(port: u16, open_browser: bool) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/api/tools", get(list_tools))
        .route("/api/run/{tool}", post(run_tool))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        .layer(middleware::from_fn(check_host));

    let url = format!("http://127.0.0.1:{port}");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        println!("PaperVault is running at {url}  (press Ctrl+C to stop)");
        if open_browser {
            let _ = open::that(&url);
        }
        axum::serve(listener, app).await?;
        Ok(())
    })
}
