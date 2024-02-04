use askama::Template;
use axum::{
    extract::Request,
    http::HeaderMap,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use notify::Watcher;
use std::path::Path;
use std::time::Duration;
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tower_livereload::LiveReloadLayer;
use tracing::info;
use tracing::Span;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("RUST_LOG", "debug");
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "with_axum_htmx_askama=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Initializing router...");

    let assets_path = std::env::current_dir().unwrap();
    let live_reload = LiveReloadLayer::new();
    let reloader = live_reload.reloader();
    let router = Router::new()
        .route("/", get(home))
        .route("/another-page", get(another_page))
        .nest_service(
            "/assets",
            ServeDir::new(format!("{}/assets", assets_path.to_str().unwrap())),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|_request: &Request<_>| tracing::debug_span!("http-request"))
                .on_request(|request: &Request<_>, _span: &Span| {
                    tracing::info!("started {} {}", request.method(), request.uri().path())
                })
                .on_response(|_response: &Response<_>, latency: Duration, _span: &Span| {
                    tracing::info!("response generated in {:?}", latency)
                })
                .on_body_chunk(|chunk: &Bytes, _latency: Duration, _span: &Span| {
                    tracing::debug!("sending {} bytes", chunk.len())
                })
                .on_eos(
                    |_trailers: Option<&HeaderMap>, stream_duration: Duration, _span: &Span| {
                        tracing::debug!("stream closed after {:?}", stream_duration)
                    },
                )
                .on_failure(
                    |_error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                        tracing::error!("something went wrong")
                    },
                ),
        )
        .layer(live_reload);

    // handling live reloading
    let mut watcher = notify::recommended_watcher(move |_| reloader.reload())?;
    let watcher_template_path_str = format!("{}/templates", assets_path.to_str().unwrap());
    let watcher_template_path = Path::new(watcher_template_path_str.as_str());
    let watcher_assets_path_str = format!("{}/assets", assets_path.to_str().unwrap());
    let watcher_assets_path = Path::new(watcher_assets_path_str.as_str());
    watcher.watch(watcher_template_path, notify::RecursiveMode::Recursive)?;
    watcher.watch(watcher_assets_path, notify::RecursiveMode::Recursive)?;

    let port = 8080_u16;
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    info!("Router initialized, now listening on port {}", port);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();

    Ok(())
}

async fn home() -> impl IntoResponse {
    let template = HomeTemplate {};
    HtmlTemplate(template)
}

async fn another_page() -> impl IntoResponse {
    let template = AnotherPageTemplate {};
    HtmlTemplate(template)
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate;

#[derive(Template)]
#[template(path = "another-page.html")]
struct AnotherPageTemplate;

struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template. Error: {}", err),
            )
                .into_response(),
        }
    }
}
