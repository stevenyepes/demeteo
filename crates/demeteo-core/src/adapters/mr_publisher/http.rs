use async_trait::async_trait;

/// The HTTP abstraction. Lets us inject a fake for tests; in
/// production this is `ReqwestHttp`.
#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, String>;
    async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HttpResponse, String>;
}

/// HTTP response. Body is always captured as text so we can log it
/// when the provider returns an error.
///
/// `headers` exists for the one thing the status code cannot say: a GitHub 403
/// is a bad token or an exhausted quota depending only on `Retry-After` and
/// `X-RateLimit-Remaining`, and
/// [`classify_list_response`](crate::domain::mr_list_error::classify_list_response)
/// is the reader. Kept as pairs rather than a map because that is what the
/// classifier takes and a listing has a handful of them.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

pub struct ReqwestHttp;

#[async_trait]
impl HttpClient for ReqwestHttp {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        let mut req = client.post(url).json(body);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Git provider request failed: {}", e))?;
        Ok(capture(resp).await)
    }

    async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HttpResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        let mut req = client.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Git provider request failed: {}", e))?;
        Ok(capture(resp).await)
    }
}

async fn capture(resp: reqwest::Response) -> HttpResponse {
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body = resp.text().await.unwrap_or_default();
    HttpResponse {
        status,
        body,
        headers,
    }
}
