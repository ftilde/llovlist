use kauf_core::ShoppingList;

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
        .post(url)
        .bearer_auth(token)
        .json(&local)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("wrong token".to_owned());
    }
    if !status.is_success() {
        return Err(format!("server said {status}"));
    }

    response.json().await.map_err(|error| format!("unreadable answer: {error}"))
}
