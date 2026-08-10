use std::time::Duration;

use kauf_core::ShoppingList;

// Long enough for a slow phone radio, short enough that a firewalled host reports
// back instead of looking like a hang.
const TIMEOUT: Duration = Duration::from_secs(10);

// The whole list goes up and the merged list comes back, so a retry or a duplicate
// costs nothing and there is no cursor to keep.
pub async fn exchange(
    server: &str,
    list: &str,
    token: &str,
    local: ShoppingList,
) -> Result<ShoppingList, String> {
    if server.is_empty() {
        return Err("set a server address first".to_owned());
    }
    let url = format!("{}/api/lists/{list}/sync", server.trim_end_matches('/'));

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

    response.json().await.map_err(|error| format!("unreadable answer: {}", describe(error)))
}

// reqwest prints only its outermost layer, which hides whether a failure was DNS, a
// refused connection or TLS -- exactly what you need to know. On wasm each layer also
// carries a full JS stack trace, so only the first line of each is kept.
fn describe(error: reqwest::Error) -> String {
    let mut message = first_line(&error.to_string());
    let mut cause = std::error::Error::source(&error);
    while let Some(error) = cause {
        message.push_str(": ");
        message.push_str(&first_line(&error.to_string()));
        cause = error.source();
    }
    message
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_owned()
}
