use std::time::Duration;

use dioxus::prelude::*;
use llovlist_core::Lists;

use crate::notice::{Notice, show};
use crate::state::{Settings, State, store};

// Without this a firewalled host looks like a hang rather than a failure.
const TIMEOUT: Duration = Duration::from_secs(10);

pub async fn exchange(server: &str, token: &str, local: Lists) -> Result<Lists, String> {
    if server.is_empty() {
        return Err("set a server address first".to_owned());
    }
    let url = format!("{}/api/sync", server.trim_end_matches('/'));

    let response = reqwest::Client::new()
        .post(&url)
        .timeout(TIMEOUT)
        .bearer_auth(token)
        .json(&local)
        .send()
        .await
        .map_err(describe)?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("wrong token".to_owned());
    }
    if !status.is_success() {
        return Err(format!("server said {status}"));
    }

    response
        .json()
        .await
        .map_err(|error| format!("unreadable answer: {}", describe(error)))
}

// reqwest's Display shows only its outermost layer; whether it was DNS, a refused
// connection or TLS is in source().
fn describe(error: reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(error) = cause {
        message.push_str(": ");
        message.push_str(&error.to_string());
        cause = error.source();
    }
    message
}

pub fn start_sync(mut state: Signal<State>, settings: Signal<Settings>, notice: Signal<Notice>) {
    let settings = settings.read().clone();
    let local = state.read().lists.clone();
    spawn(async move {
        match exchange(&settings.server, &settings.token, local).await {
            Ok(merged) => {
                state.write().lists.merge(merged);
                store(state);
                let count = state.read().lists.names().len();
                let plural = if count == 1 { "" } else { "s" };
                show(notice, format!("synced, {count} list{plural}"));
            }
            Err(reason) => {
                eprintln!("sync failed: {reason}");
                show(notice, format!("sync failed: {reason}"));
            }
        }
    });
}
