//! Whether the Hub shows what an instance reported. The [module
//! doc](super) says which fields that covers and why a refusal here is not
//! enforcement. Queueing a request is decided by
//! [`enqueue_verdict`](crate::socket::requests::enqueue_verdict) instead,
//! which needs the scope of the request's kind alone.

use crate::auth::handlers::ApiError;
use demeteo_hub_protocol::Scope;

/// `Ok` when the instance last reported `read` among `granted`.
pub fn read_verdict(granted: &[Scope]) -> Result<(), ApiError> {
    if granted.contains(&Scope::Read) {
        Ok(())
    } else {
        Err(ApiError::scope_not_granted(Scope::Read))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn read_is_granted_when_the_instance_reports_it_beside_any_other() {
        assert_eq!(read_verdict(&[Scope::Read]), Ok(()));
        assert_eq!(
            read_verdict(&[Scope::Spend, Scope::Read, Scope::Gates]),
            Ok(())
        );
    }

    #[tokio::test]
    async fn read_is_refused_by_name_when_the_instance_does_not_report_it() {
        for granted in [&[][..], &[Scope::Spend, Scope::Gates][..]] {
            let refusal = read_verdict(granted).unwrap_err();
            let response = refusal.into_response();
            assert_eq!(response.status(), StatusCode::CONFLICT, "{granted:?}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(
                body, r#"{"error":"scope_not_granted","scope":"read"}"#,
                "{granted:?}"
            );
        }
    }
}
