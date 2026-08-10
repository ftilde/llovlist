use std::time::Duration;

use kauf_core::ShoppingList;

// Long enough for a slow phone radio, short enough that a firewalled host reports
// back instead of looking like a hang.
const TIMEOUT: Duration = Duration::from_secs(10);

// Blocking on purpose: it runs on a worker thread and the result is handed back
// through the Slint event loop, which is simpler than bridging an async runtime.
pub fn exchange(
    server: &str,
    list: &str,
    token: &str,
    local: ShoppingList,
) -> Result<ShoppingList, String> {
    if server.is_empty() {
        return Err("set a server address first".to_owned());
    }
    let url = format!("{}/api/lists/{list}/sync", server.trim_end_matches('/'));
    log::info!("posting {} items to {url}", local.items().len());

    let response = reqwest::blocking::Client::new()
        .post(&url)
        .timeout(TIMEOUT)
        .bearer_auth(token)
        .json(&local)
        .send()
        .map_err(describe)?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("wrong token".to_owned());
    }
    if !status.is_success() {
        return Err(format!("server said {status}"));
    }

    response.json().map_err(|error| format!("unreadable answer: {}", describe(error)))
}

// reqwest prints only its outermost layer, which hides whether a failure was DNS, a
// refused connection or TLS -- exactly what you need to know.
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
