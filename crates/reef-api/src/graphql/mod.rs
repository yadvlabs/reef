//! GitHub GraphQL v4 subset. Schema-backed so introspection works (gh runs `__type` queries
//! against enterprise hosts to detect features).

pub mod issues;
pub mod mutation;
pub mod projects;
pub mod pulls;
pub mod query;
pub mod types;

use crate::AppState;
use crate::auth::Auth;
use async_graphql::http::GraphiQLSource;
use async_graphql::{EmptySubscription, Error as GqlError, ErrorExtensions};
use async_graphql_axum::GraphQLRequest;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use reef_db::Db;
use reef_git::Store;
use std::sync::Arc;

pub type Schema = async_graphql::Schema<query::Query, mutation::Mutation, EmptySubscription>;

pub fn build_schema(db: Db, store: Store, config: Arc<crate::Config>) -> Schema {
    Schema::build(query::Query, mutation::Mutation::default(), EmptySubscription)
        .data(db)
        .data(store)
        .data(config)
        .limit_depth(24)
        .limit_complexity(5000)
        .finish()
}

pub async fn handler(
    State(s): State<AppState>,
    auth: Auth,
    key: Option<axum::extract::Extension<crate::ratelimit::RateKey>>,
    req: GraphQLRequest,
) -> impl IntoResponse {
    let req = req.into_inner().data(auth).data(key.map(|k| k.0)).data(crate::AppStateHandle(s.clone()));
    let resp = s.schema.execute(req).await;
    // GitHub places the error `type` (NOT_FOUND, FORBIDDEN, ...) at the top level of each error
    // object, next to `message`, and clients such as gh match on it there.
    let mut body = serde_json::to_value(&resp).unwrap_or_default();
    if let Some(errors) = body.get_mut("errors").and_then(|e| e.as_array_mut()) {
        for e in errors {
            let kind = e.get("extensions").and_then(|x| x.get("type")).cloned();
            if let (Some(kind), Some(obj)) = (kind, e.as_object_mut()) {
                obj.insert("type".into(), kind);
            }
        }
    }
    let mut r = axum::Json(body).into_response();
    for (k, v) in resp.http_headers.iter() {
        r.headers_mut().append(k.clone(), v.clone());
    }
    r
}

/// The full application state, available to resolvers that call shared services.
pub fn app(ctx: &async_graphql::Context<'_>) -> AppState {
    ctx.data_unchecked::<crate::AppStateHandle>().0.clone()
}

pub async fn playground() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/api/graphql").title("Reef GraphQL").finish())
}

/// GitHub-style typed errors: `{"type": "NOT_FOUND", "message": "..."}`.
pub fn gql_error(kind: &str, msg: impl Into<String>) -> GqlError {
    let k = kind.to_string();
    GqlError::new(msg.into()).extend_with(|_, e| e.set("type", k.clone()))
}

pub fn not_found(msg: impl Into<String>) -> GqlError {
    gql_error("NOT_FOUND", msg)
}

pub fn forbidden(msg: impl Into<String>) -> GqlError {
    gql_error("FORBIDDEN", msg)
}

pub fn from_core(e: reef_core::Error) -> GqlError {
    match e {
        reef_core::Error::NotFound => not_found("Not found"),
        reef_core::Error::Unauthorized(m) => gql_error("UNAUTHORIZED", m),
        reef_core::Error::Forbidden(m) => forbidden(m),
        reef_core::Error::Validation { message, errors } => {
            let detail = errors.iter().filter_map(|f| f.message.clone()).collect::<Vec<_>>().join("; ");
            gql_error("UNPROCESSABLE", if detail.is_empty() { message } else { detail })
        }
        reef_core::Error::Conflict(m) => gql_error("UNPROCESSABLE", m),
        reef_core::Error::Internal(e) => {
            tracing::error!(error = %e, "graphql internal error");
            gql_error("INTERNAL", "Something went wrong")
        }
    }
}

/// Shared per-request context helpers.
pub struct Ctx<'a> {
    pub db: &'a Db,
    pub store: &'a Store,
    pub config: &'a Arc<crate::Config>,
    pub auth: &'a Auth,
}

impl<'a> Ctx<'a> {
    pub fn from(ctx: &'a async_graphql::Context<'_>) -> Ctx<'a> {
        Ctx {
            db: ctx.data_unchecked::<Db>(),
            store: ctx.data_unchecked::<Store>(),
            config: ctx.data_unchecked::<Arc<crate::Config>>(),
            auth: ctx.data_unchecked::<Auth>(),
        }
    }
    pub fn viewer(&self) -> Option<&reef_db::accounts::Account> {
        self.auth.user.as_ref()
    }
}
