//! Test-only helpers: a throwaway HTTP server on a free port.
//!
//! Behind the `testing` feature rather than `#[cfg(test)]` so the integration
//! tests in `tests/`, which link the library as an ordinary dependency, can
//! stand up the same servers the unit tests do.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::Response;

/// A server that answers every path with whatever `handler` returns.
///
/// Shuts down when the returned value is dropped, so a test never leaks a port.
pub struct TestServer {
    pub base_url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

pub async fn serve<F>(handler: F) -> TestServer
where
    F: Fn(Request) -> Response<Body> + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let app = axum::Router::new().fallback(move |request: Request| {
        let handler = handler.clone();
        async move { handler(request) }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    TestServer { base_url: format!("http://127.0.0.1:{port}"), shutdown: Some(tx) }
}

pub fn html(body: impl Into<String>) -> Response<Body> {
    with_type(body, "text/html")
}

pub fn with_type(body: impl Into<String>, content_type: &str) -> Response<Body> {
    status(200, body, content_type)
}

pub fn status(code: u16, body: impl Into<String>, content_type: &str) -> Response<Body> {
    Response::builder()
        .status(code)
        .header("content-type", content_type)
        .body(Body::from(body.into()))
        .expect("response")
}

pub fn redirect(location: &str) -> Response<Body> {
    Response::builder().status(302).header("location", location).body(Body::empty()).expect("response")
}

/// The real `obscura` binary, if it is installed on this machine.
///
/// Tests that need a genuine JavaScript engine skip themselves when it is
/// absent, so the suite still passes on a machine without it.
pub fn installed_obscura() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("obscura")).find(|candidate| candidate.is_file())
}
