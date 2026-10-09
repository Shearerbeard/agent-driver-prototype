//! Bounded retry layer for live HTTP calls (summarizer + judge).
//!
//! Transport errors are retried with exponential backoff and light jitter.
//! HTTP 429 is retried once if the endpoint returns a parseable `Retry-After`
//! header; otherwise it is classified as exhaustion. Other non-2xx statuses
//! fail loud immediately.

use std::time::Duration;

use rand::Rng as _;
use reqwest::header::HeaderMap;

/// Maximum attempts for transport-level failures.
///
/// Sequence: initial attempt + 3 retries, with backoff delays of 500 ms,
/// 2 s, and 8 s between attempts. After the final attempt fails the error is
/// classified as exhaustion so the runner halts and writes a resume.
const MAX_ATTEMPTS: usize = 4;

/// Backoff delays between transport-error retries.
const BACKOFF_DELAYS: [Duration; MAX_ATTEMPTS - 1] = [
    Duration::from_millis(500),
    Duration::from_secs(2),
    Duration::from_secs(8),
];

/// Maximum jitter added to a backoff delay (25% of the delay, capped).
const JITTER_CAP_MS: u64 = 250;

/// Execute a request with bounded retry and 429 handling.
///
/// On success the response is returned. On terminal failure an `anyhow::Error`
/// is returned with a message that the runner's exhaustion detector can
/// recognise.
pub async fn execute_with_retry(
    client: &reqwest::Client,
    request: reqwest::Request,
    service: &str,
) -> anyhow::Result<reqwest::Response> {
    let mut attempt = 0usize;

    loop {
        let req = request
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("{service} request cannot be cloned for retry"))?;
        let response = client.execute(req).await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return handle_429(resp, client, request, service, attempt).await;
                }
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!(
                        "{service} request failed: HTTP {status}: {body}"
                    ));
                }
                return Ok(resp);
            }
            Err(e) => {
                if attempt == MAX_ATTEMPTS - 1 {
                    return Err(anyhow::anyhow!("{service} exhausted: {e}"));
                }
                let delay = backoff_delay(attempt);
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

async fn handle_429(
    response: reqwest::Response,
    client: &reqwest::Client,
    request: reqwest::Request,
    service: &str,
    attempt: usize,
) -> anyhow::Result<reqwest::Response> {
    let wait = retry_after(response.headers());
    let Some(wait) = wait else {
        return Err(anyhow::anyhow!("{service} exhausted: 429 rate limited"));
    };

    // Only one wait-and-retry for 429. If this is already the second 429
    // (attempt > 0) or the wait header is missing, halt.
    if attempt > 0 {
        return Err(anyhow::anyhow!("{service} exhausted: repeated 429"));
    }

    tokio::time::sleep(wait).await;

    let req = request
        .try_clone()
        .ok_or_else(|| anyhow::anyhow!("{service} request cannot be cloned for 429 retry"))?;
    let retry = client.execute(req).await;
    match retry {
        Ok(resp) => {
            let status = resp.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(anyhow::anyhow!("{service} exhausted: 429 after retry"));
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "{service} request failed: HTTP {status}: {body}"
                ));
            }
            Ok(resp)
        }
        Err(e) => Err(anyhow::anyhow!(
            "{service} exhausted: 429 retry failed: {e}"
        )),
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds: u64 = value.parse().ok()?;
    Some(Duration::from_secs(seconds))
}

fn backoff_delay(attempt: usize) -> Duration {
    let base = BACKOFF_DELAYS
        .get(attempt)
        .copied()
        .unwrap_or(BACKOFF_DELAYS[BACKOFF_DELAYS.len() - 1]);
    let jitter_ms = rand::thread_rng()
        .gen_range(0..=JITTER_CAP_MS)
        .min(base.as_millis() as u64 / 4);
    base + Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;

    struct FlakyTransportResponder {
        attempt: AtomicUsize,
        success_body: String,
    }

    impl Respond for FlakyTransportResponder {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let n = self.attempt.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                // Simulate a dropped/never-completed connection by delaying
                // longer than the client's timeout.
                ResponseTemplate::new(200).set_delay(Duration::from_secs(10))
            } else {
                ResponseTemplate::new(200).set_body_string(self.success_body.clone())
            }
        }
    }

    struct RetryAfterResponder {
        attempt: AtomicUsize,
        success_body: String,
    }

    impl Respond for RetryAfterResponder {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let n = self.attempt.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "1")
                    .set_body_string("rate limited")
            } else {
                ResponseTemplate::new(200).set_body_string(self.success_body.clone())
            }
        }
    }

    fn short_timeout_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn build_request(client: &reqwest::Client, url: &str) -> reqwest::Request {
        client.get(url).build().unwrap_or_else(|e| panic!("{e}"))
    }

    #[tokio::test]
    async fn transport_error_retried_then_succeeds() {
        let server = MockServer::start().await;
        let responder = FlakyTransportResponder {
            attempt: AtomicUsize::new(0),
            success_body: "ok".to_owned(),
        };
        Mock::given(path("/flaky"))
            .respond_with(responder)
            .expect(3)
            .mount(&server)
            .await;

        let client = short_timeout_client();
        let request = build_request(&client, &format!("{}/flaky", server.uri()));
        let response = execute_with_retry(&client, request, "test")
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap_or_default(), "ok");
    }

    #[tokio::test]
    async fn retry_after_429_retries_once_then_succeeds() {
        let server = MockServer::start().await;
        let responder = RetryAfterResponder {
            attempt: AtomicUsize::new(0),
            success_body: "ok".to_owned(),
        };
        Mock::given(path("/ratelimited"))
            .respond_with(responder)
            .expect(2)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let request = build_request(&client, &format!("{}/ratelimited", server.uri()));
        let response = execute_with_retry(&client, request, "test")
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap_or_default(), "ok");
    }

    #[tokio::test]
    async fn transport_errors_exhaust_after_max_attempts() {
        let server = MockServer::start().await;
        Mock::given(path("/always-down"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .expect(4)
            .mount(&server)
            .await;

        let client = short_timeout_client();
        let request = build_request(&client, &format!("{}/always-down", server.uri()));
        let err = match execute_with_retry(&client, request, "test").await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected exhaustion"),
        };

        assert!(err.contains("exhausted"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn non_retryable_non_2xx_fails_loud() {
        let server = MockServer::start().await;
        Mock::given(path("/bad"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let request = build_request(&client, &format!("{}/bad", server.uri()));
        let err = match execute_with_retry(&client, request, "test").await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("HTTP 500"), "unexpected error: {err}");
    }
}
