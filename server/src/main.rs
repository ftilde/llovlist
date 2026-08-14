mod auth;
mod store;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::post;
use clap::Parser;
use kauf_core::Lists;

use auth::{Authenticated, Token, TokenHash};
use store::Store;

#[derive(Parser)]
#[command(about, version)]
enum Command {
    GenToken,
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
        // Via the environment by preference: an argument would be visible in /proc.
        #[arg(long, env = "KAUF_TOKEN_HASH")]
        token_hash: TokenHash,
        #[arg(long, default_value = "lists")]
        state_dir: PathBuf,
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
        Command::Serve {
            addr,
            token_hash,
            state_dir,
        } => {
            let state = AppState {
                store: Arc::new(Store::open(state_dir)?),
                token_hash,
            };

            let listener = tokio::net::TcpListener::bind(addr).await?;
            println!("listening on http://{}", listener.local_addr()?);
            axum::serve(listener, router(state)).await?;
        }
    }
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/sync", post(sync))
        .with_state(state)
}

async fn sync(
    _: Authenticated,
    State(state): State<AppState>,
    Json(incoming): Json<Lists>,
) -> Result<Json<Lists>, StatusCode> {
    state.store.sync(incoming).map(Json).map_err(|error| {
        eprintln!("sync failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use kauf_core::{DeviceId, Edit, ItemName, ListName};
    use tower::ServiceExt;

    use super::*;

    fn serve(dir: &tempfile::TempDir) -> (Router, Token) {
        let token = Token::generate();
        let state = AppState {
            store: Arc::new(Store::open(dir.path().to_owned()).unwrap()),
            token_hash: token.hash(),
        };
        (router(state), token)
    }

    fn lists_with(device: u64, list: &str, item: &str) -> Lists {
        let mut lists = Lists::default();
        let list = ListName::new(list).unwrap();
        let key = lists.item_key_at_end(DeviceId::new(device), &list);
        lists.edit(
            DeviceId::new(device),
            Edit::add_item(list, ItemName::new(item).unwrap(), key),
        );
        lists
    }

    async fn post(app: &Router, bearer: &str, lists: &Lists) -> (StatusCode, Body) {
        let request = Request::post("/api/sync")
            .header("authorization", format!("Bearer {bearer}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(lists).unwrap()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        (response.status(), response.into_body())
    }

    async fn names(body: Body) -> Vec<String> {
        let lists: Lists =
            serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
        lists
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    #[tokio::test]
    async fn two_devices_see_each_others_lists() {
        let dir = tempfile::tempdir().unwrap();
        let (app, token) = serve(&dir);

        let _ = post(&app, &token.to_string(), &lists_with(1, "home", "Milch")).await;
        let (status, body) = post(
            &app,
            &token.to_string(),
            &lists_with(2, "baumarkt", "Schrauben"),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(names(body).await, ["baumarkt", "home"]);
    }

    #[tokio::test]
    async fn a_wrong_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _) = serve(&dir);

        let (status, _) = post(&app, &"0".repeat(32), &Lists::default()).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
