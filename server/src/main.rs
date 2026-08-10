mod auth;
mod store;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{FromRef, Path, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, StatusCode};
use axum::routing::post;
use clap::Parser;
use kauf_core::ShoppingList;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use auth::{Authenticated, Token, TokenHash};
use store::{ListId, Store};

#[derive(Parser)]
#[command(about, version)]
enum Command {
    GenToken,
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
        #[arg(long)]
        token_hash: TokenHash,
        #[arg(long, default_value = "lists")]
        state_dir: PathBuf,
        #[arg(long)]
        web_dir: Option<PathBuf>,
    },
}

#[derive(Clone)]
struct AppState {
    store: Arc<Store>,
    token_hash: TokenHash,
}

impl FromRef<AppState> for TokenHash {
    fn from_ref(state: &AppState) -> Self {
        state.token_hash.clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Command::parse() {
        Command::GenToken => {
            let token = Token::generate();
            println!("token (give to each client): {token}");
            println!("hash  (pass to serve):       {}", token.hash());
        }
        Command::Serve { addr, token_hash, state_dir, web_dir } => {
            let state = AppState { store: Arc::new(Store::open(state_dir)?), token_hash };

            let listener = tokio::net::TcpListener::bind(addr).await?;
            println!("listening on http://{}", listener.local_addr()?);
            axum::serve(listener, router(state, web_dir)).await?;
        }
    }
    Ok(())
}

fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    // Any origin is fine: the credential is a bearer token, so a foreign page that
    // does not have it gets a 401, and one that does was given it on purpose.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::POST])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);
    let api =
        Router::new().route("/api/lists/{list}/sync", post(sync)).layer(cors).with_state(state);
    match web_dir {
        // Unknown paths fall back to the app shell, so deep links work.
        Some(dir) => {
            let shell = ServeFile::new(dir.join("index.html"));
            api.fallback_service(ServeDir::new(dir).fallback(shell))
        }
        None => api,
    }
}

async fn sync(
    _: Authenticated,
    State(state): State<AppState>,
    Path(list): Path<ListId>,
    Json(incoming): Json<ShoppingList>,
) -> Result<Json<ShoppingList>, StatusCode> {
    state.store.sync(&list, incoming).map(Json).map_err(|error| {
        eprintln!("sync failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use kauf_core::{DeviceId, Edit, ItemName};
    use tower::ServiceExt;

    use super::*;

    fn serve(dir: &tempfile::TempDir) -> (Router, Token) {
        let token = Token::generate();
        let state = AppState {
            store: Arc::new(Store::open(dir.path().to_owned()).unwrap()),
            token_hash: token.hash(),
        };
        (router(state, None), token)
    }

    fn list_with(device: u64, item: &str) -> ShoppingList {
        let mut list = ShoppingList::default();
        list.edit(DeviceId::new(device), Edit::add_item(ItemName::new(item).unwrap()));
        list
    }

    async fn post(
        app: &Router,
        path: &str,
        bearer: &str,
        list: &ShoppingList,
    ) -> (StatusCode, Body) {
        let request = Request::post(path)
            .header("authorization", format!("Bearer {bearer}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(list).unwrap()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        (response.status(), response.into_body())
    }

    async fn names(body: Body) -> Vec<String> {
        let list: ShoppingList =
            serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
        list.items().into_iter().map(|(name, _)| name.to_string()).collect()
    }

    #[tokio::test]
    async fn two_devices_see_each_others_items() {
        let dir = tempfile::tempdir().unwrap();
        let (app, token) = serve(&dir);
        let path = "/api/lists/home/sync";

        let _ = post(&app, path, &token.to_string(), &list_with(1, "Milch")).await;
        let (status, body) = post(&app, path, &token.to_string(), &list_with(2, "Brot")).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(names(body).await, ["Brot", "Milch"]);
    }

    #[tokio::test]
    async fn a_wrong_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = serve(&dir);

        let (status, _) =
            post(&app, "/api/lists/home/sync", &"0".repeat(32), &ShoppingList::default()).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // The character set lives in ListId, but only `Path<ListId>` makes axum enforce it.
    #[tokio::test]
    async fn list_names_cannot_escape_the_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (app, token) = serve(&dir);

        for name in ["..%2Fescape", "..", "escape.json", ""] {
            let path = format!("/api/lists/{name}/sync");
            let (status, _) = post(&app, &path, &token.to_string(), &ShoppingList::default()).await;
            assert_ne!(status, StatusCode::OK, "accepted list name {name:?}");
        }
    }
}
