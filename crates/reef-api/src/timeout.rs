//! The request timeout and how routes opt out of it.
//!
//! [`request_timeout`] wraps every route: a request that has not answered after
//! [`REQUEST_TIMEOUT`] gets 408. Only routes that stream or long-poll opt out, *where they are
//! defined*: a whole sub-router with [`exempt_router`] (git smart HTTP, LFS, the runner
//! protocol, the health endpoints), a single handler with [`exempt`] (source archives). The
//! marker is a per-route layer that flips a flag the guard put on the request, so a new route
//! that is not marked is timed by default and nothing has to be kept in step in a central
//! list. The decision is recorded on the response as [`TimeoutPolicy`] (tests read it; it is
//! not sent).
//!
//! Work that must not be cut half way (git writes followed by the bookkeeping that goes with
//! them: merges, repository create/delete/fork, ref and file writes, imports, webhook test
//! deliveries) does not opt out of the timeout. It runs through [`detached`] in the shared
//! helper that REST and GraphQL both call, so the deadline, a client that hangs up, or any
//! other drop of the request future only stops the *wait*: the work finishes on its own task.

use axum::Router;
use axum::extract::Request;
use axum::handler::{Handler, Layered};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tower::{Layer, Service};

/// Requests on API and UI routes must answer within this long.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Run `work` on its own task and wait for it. Dropping the returned future (the request
/// timed out, or the client went away) does not cancel `work`: it runs to completion and its
/// result is discarded. A panic in `work` is re-raised here, so the panic layer still answers
/// 500 for the request that started it.
pub async fn detached<T, F>(work: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::spawn(work).await {
        Ok(v) => v,
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        // Only a runtime shutting down cancels a spawned task.
        Err(e) => panic!("detached work was cancelled: {e}"),
    }
}

/// The flag the guard attaches to a request; exempt routes set it as the request reaches them.
#[derive(Clone, Debug, Default)]
pub struct TimeoutCell(Arc<AtomicBool>);

impl TimeoutCell {
    pub fn exempt(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// What the guard did with a request, left on the response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeoutPolicy {
    /// The route opted out; no deadline applied.
    pub exempt: bool,
    /// The deadline passed and the request was answered 408.
    pub timed_out: bool,
}

/// The marker layer: flips the request's `TimeoutCell` and passes the request on.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExemptLayer;

impl<S> Layer<S> for ExemptLayer {
    type Service = Exempt<S>;
    fn layer(&self, inner: S) -> Exempt<S> {
        Exempt { inner }
    }
}

#[derive(Clone, Debug)]
pub struct Exempt<S> {
    inner: S,
}

impl<S, B> Service<Request<B>> for Exempt<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> S::Future {
        if let Some(cell) = req.extensions().get::<TimeoutCell>() {
            cell.0.store(true, Ordering::Relaxed);
        }
        self.inner.call(req)
    }
}

/// Mark one handler as exempt: `put(exempt(contents::put_contents))`.
pub fn exempt<H, T, S>(handler: H) -> Layered<ExemptLayer, H, T, S>
where
    H: Handler<T, S> + Clone + Send + 'static,
    S: Clone + Send + Sync + 'static,
    T: 'static,
{
    handler.layer(ExemptLayer)
}

/// Mark every route of a sub-router as exempt.
pub fn exempt_router<S: Clone + Send + Sync + 'static>(router: Router<S>) -> Router<S> {
    router.route_layer(ExemptLayer)
}

/// The guard: `REQUEST_TIMEOUT` for every route that has not opted out; 408 on expiry. The
/// exemption is read when the deadline fires (the marker sits inside this layer, after
/// routing and the rate limiter), so an exempt route is never cut short.
pub async fn request_timeout(mut req: Request, next: Next) -> Response {
    let cell = TimeoutCell::default();
    req.extensions_mut().insert(cell.clone());
    let fut = next.run(req);
    tokio::pin!(fut);
    let sleep = tokio::time::sleep(REQUEST_TIMEOUT);
    tokio::pin!(sleep);
    let mut res = tokio::select! {
        res = &mut fut => res,
        _ = &mut sleep => {
            if cell.exempt() {
                fut.await
            } else {
                let mut res = (
                    StatusCode::REQUEST_TIMEOUT,
                    [(header::CONTENT_TYPE, "application/json")],
                    r#"{"message":"request timed out"}"#,
                )
                    .into_response();
                res.extensions_mut().insert(TimeoutPolicy { exempt: false, timed_out: true });
                return res;
            }
        }
    };
    res.extensions_mut().insert(TimeoutPolicy { exempt: cell.exempt(), timed_out: false });
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    async fn slow() -> &'static str {
        tokio::time::sleep(Duration::from_millis(200)).await;
        "done"
    }

    async fn quick() -> &'static str {
        "done"
    }

    /// The guard with a short deadline, for the test only.
    async fn short_timeout(mut req: Request, next: Next) -> Response {
        let cell = TimeoutCell::default();
        req.extensions_mut().insert(cell.clone());
        match tokio::time::timeout(Duration::from_millis(50), next.run(req)).await {
            Ok(mut res) => {
                res.extensions_mut().insert(TimeoutPolicy { exempt: cell.exempt(), timed_out: false });
                res
            }
            Err(_) if cell.exempt() => panic!("an exempt route was cut short"),
            Err(_) => {
                let mut res = StatusCode::REQUEST_TIMEOUT.into_response();
                res.extensions_mut().insert(TimeoutPolicy { exempt: false, timed_out: true });
                res
            }
        }
    }

    /// Detached work outlives the request that started it: the deadline answers 408, the work
    /// still runs to the end (a merge is never left with the branch moved but the pull request
    /// open). A panic inside it still reaches the caller.
    #[tokio::test]
    async fn detached_work_is_not_cancelled_by_the_deadline() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let app = Router::new()
            .route(
                "/write",
                get(move || {
                    let flag = flag.clone();
                    async move {
                        detached(async move {
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            flag.store(true, Ordering::SeqCst);
                        })
                        .await;
                        "written"
                    }
                }),
            )
            .layer(axum::middleware::from_fn(short_timeout));
        let res = app.oneshot(Request::get("/write").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT, "the caller stops waiting at the deadline");
        assert!(!done.load(Ordering::SeqCst));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(done.load(Ordering::SeqCst), "the work finished anyway");
        let caught = tokio::spawn(detached(async { panic!("boom") })).await.unwrap_err();
        assert!(caught.is_panic(), "a panic in detached work is re-raised in the caller");
    }

    #[tokio::test]
    async fn markers_flip_the_cell_before_the_handler_runs() {
        let app = Router::new()
            .route("/timed", get(quick))
            .route("/handler", get(exempt(slow)))
            .merge(exempt_router(Router::new().route("/router/{id}", get(slow))))
            .route("/slow", get(slow))
            .layer(axum::middleware::from_fn(short_timeout));
        let policy = |app: Router, path: &str| {
            let req = Request::get(path).body(Body::empty()).unwrap();
            async move {
                let res = app.oneshot(req).await.unwrap();
                (res.status(), *res.extensions().get::<TimeoutPolicy>().unwrap())
            }
        };
        assert_eq!(policy(app.clone(), "/timed").await, (StatusCode::OK, TimeoutPolicy { exempt: false, timed_out: false }));
        assert_eq!(policy(app.clone(), "/slow").await, (StatusCode::REQUEST_TIMEOUT, TimeoutPolicy { exempt: false, timed_out: true }));
        // Exempt routes outlive the deadline; the real guard waits for them.
        let app2 = Router::new()
            .route("/handler", get(exempt(slow)))
            .merge(exempt_router(Router::new().route("/router/{id}", get(slow))))
            .layer(axum::middleware::from_fn(request_timeout));
        let policy2 = |path: &str| {
            let app = app2.clone();
            let req = Request::get(path).body(Body::empty()).unwrap();
            async move {
                let res = app.oneshot(req).await.unwrap();
                (res.status(), *res.extensions().get::<TimeoutPolicy>().unwrap())
            }
        };
        assert_eq!(policy2("/handler").await, (StatusCode::OK, TimeoutPolicy { exempt: true, timed_out: false }));
        assert_eq!(policy2("/router/7").await, (StatusCode::OK, TimeoutPolicy { exempt: true, timed_out: false }));
        // A route that nobody marked is timed: the safe default.
        assert!(!policy(app, "/timed").await.1.exempt);
    }
}
