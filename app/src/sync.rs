use std::time::Duration;

use kauf_core::ShoppingList;

// Without this a firewalled host looks like a hang rather than a failure.
const TIMEOUT: Duration = Duration::from_secs(10);

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

    response
        .json()
        .await
        .map_err(|error| format!("unreadable answer: {}", describe(error)))
}

// reqwest's Display shows only its outermost layer; whether it was DNS, a refused
// connection or TLS is in source(). On wasm each layer appends a JS stack trace.
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
