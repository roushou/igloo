//! The web console's static files, served at `/` beside the REST API and MCP.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, HeaderValue};
use axum::http::{Method, Request, Response, StatusCode};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

/// The built console in a directory holding `index.html`.
///
/// As a router fallback it answers `GET` and `HEAD`: files by path, and `index.html` for any
/// other path outside `/v1` and `/mcp`, which stay 404 when no route matches. Files under
/// `/assets/` are cached immutably; everything else is revalidated on every request.
#[derive(Clone, Debug)]
pub struct WebConsole {
    dir: PathBuf,
}

impl WebConsole {
    /// Prefixes that belong to the server's other interfaces and never fall back to the console.
    const RESERVED: [&'static str; 2] = ["/v1", "/mcp"];
    /// Where the build writes content-hashed files.
    const ASSETS: &'static str = "/assets/";
    const IMMUTABLE: &'static str = "public, max-age=31536000, immutable";
    const REVALIDATE: &'static str = "no-cache";

    /// Serves `dir`, which must contain `index.html`; see [`WebConsole::check`].
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Whether `dir` holds a console build.
    #[must_use]
    pub fn check(dir: &Path) -> bool {
        dir.join("index.html").is_file()
    }

    /// Adds the console as the fallback of `router`.
    pub fn mount(self, router: Router) -> Router {
        router.fallback(move |request: Request<Body>| {
            let console = self.clone();
            async move { console.respond(request).await }
        })
    }

    async fn respond(&self, request: Request<Body>) -> Response<Body> {
        let path = request.uri().path().to_owned();
        let reserved = Self::RESERVED.iter().any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        });
        if reserved || !matches!(*request.method(), Method::GET | Method::HEAD) {
            return Self::status(StatusCode::NOT_FOUND);
        }
        let hashed = path.starts_with(Self::ASSETS);
        let response = if hashed {
            ServeDir::new(&self.dir).oneshot(request).await
        } else {
            let index = ServeFile::new(self.dir.join("index.html"));
            ServeDir::new(&self.dir)
                .fallback(index)
                .oneshot(request)
                .await
        };
        let Ok(response) = response;
        let mut response = response.map(Body::new);
        if response.status().is_success() {
            let policy = if hashed {
                Self::IMMUTABLE
            } else {
                Self::REVALIDATE
            };
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static(policy));
        }
        response
    }

    fn status(status: StatusCode) -> Response<Body> {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = status;
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::routing::get;

    use super::*;

    const INDEX: &str = "<html>console</html>";

    fn console() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), INDEX).expect("index");
        std::fs::create_dir(dir.path().join("assets")).expect("assets");
        std::fs::write(dir.path().join("assets/app-abc123.js"), "console.log(1)").expect("asset");
        std::fs::write(dir.path().join("favicon.svg"), "<svg/>").expect("favicon");
        let api = Router::new().route("/v1/repos", get(|| async { "api" }));
        let router = WebConsole::new(dir.path()).mount(api);
        (dir, router)
    }

    async fn get_path(router: &Router, method: Method, uri: &str) -> (Response<Body>, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        let response = router.clone().oneshot(request).await.expect("response");
        let (parts, body) = response.into_parts();
        let body = to_bytes(body, usize::MAX).await.expect("body");
        let text = String::from_utf8_lossy(&body).into_owned();
        (Response::from_parts(parts, Body::empty()), text)
    }

    fn cache(response: &Response<Body>) -> Option<&str> {
        response.headers().get(CACHE_CONTROL)?.to_str().ok()
    }

    #[tokio::test]
    async fn root_and_client_routes_return_the_index() {
        let (_dir, router) = console();
        for uri in ["/", "/tasks/x", "/changes/chg_1/diff"] {
            let (response, body) = get_path(&router, Method::GET, uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(body, INDEX, "{uri}");
            assert_eq!(cache(&response), Some("no-cache"), "{uri}");
        }
    }

    #[tokio::test]
    async fn files_are_served_by_path_and_hashed_assets_are_immutable() {
        let (_dir, router) = console();
        let (response, body) = get_path(&router, Method::GET, "/assets/app-abc123.js").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body, "console.log(1)");
        assert_eq!(
            cache(&response),
            Some("public, max-age=31536000, immutable")
        );
        let (response, body) = get_path(&router, Method::GET, "/favicon.svg").await;
        assert_eq!(body, "<svg/>");
        assert_eq!(cache(&response), Some("no-cache"));
    }

    #[tokio::test]
    async fn a_missing_asset_is_not_found() {
        let (_dir, router) = console();
        let (response, body) = get_path(&router, Method::GET, "/assets/gone.js").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_ne!(body, INDEX);
    }

    #[tokio::test]
    async fn the_api_stays_the_api() {
        let (_dir, router) = console();
        let (response, body) = get_path(&router, Method::GET, "/v1/repos").await;
        assert_eq!((response.status(), body.as_str()), (StatusCode::OK, "api"));
        for uri in ["/v1/nothing", "/v1", "/mcp", "/mcp/session"] {
            let (response, body) = get_path(&router, Method::GET, uri).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
            assert_ne!(body, INDEX, "{uri}");
        }
    }

    #[tokio::test]
    async fn only_get_and_head_reach_the_console() {
        let (_dir, router) = console();
        let (response, _) = get_path(&router, Method::POST, "/tasks/x").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let (response, body) = get_path(&router, Method::HEAD, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn paths_outside_the_directory_are_not_served() {
        let (_dir, router) = console();
        let (_, body) = get_path(&router, Method::GET, "/..%2f..%2fetc/passwd").await;
        assert_eq!(body, INDEX);
    }

    #[test]
    fn check_requires_an_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!WebConsole::check(dir.path()));
        std::fs::write(dir.path().join("index.html"), INDEX).expect("index");
        assert!(WebConsole::check(dir.path()));
    }
}
