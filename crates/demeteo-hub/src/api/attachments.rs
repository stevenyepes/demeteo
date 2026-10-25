//! `POST /v1/instances/{instance_id}/attachments`: a browser stages a file
//! for a dispatch to name. The custody rule it keeps is
//! [`attachments`](crate::attachments)'s; `docs/HUB.md` §6.3 is the lifecycle.
//!
//! Nothing here logs, `Debug`-prints or puts into an error the body, its
//! digest or the data key: every refusal is a literal [`ApiError`], and what
//! reaches the log is the text of a key-service or envelope error, neither of
//! which carries a byte of them.

use super::fleet::granted;
use super::runs::owned;
use super::{ApiContext, PostgresApi};
use crate::attachments::envelope::{self, EnvelopeError};
use crate::attachments::postgres::PostgresAttachments;
use crate::auth::handlers::{store_failed, unix_now, ApiError, CurrentSession};
use crate::routes::AppState;
use crate::store::attachments::{NewAttachment, WrappedKey};
use crate::store::UserId;
use axum::body::Bytes;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::StatusCode;
use axum::Json;
use demeteo_hub_protocol::Scope;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

/// A staged attachment as its uploader is told of it. `size` and `sha256`
/// describe the content, so the browser can check them against the file it
/// sent; `Debug` leaves the digest out.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct UploadedView {
    pub attachment_id: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    /// Unix seconds.
    pub expires_at: u64,
}

impl fmt::Debug for UploadedView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadedView")
            .field("attachment_id", &self.attachment_id)
            .field("size", &self.size)
            .field("sha256", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Stages `bytes` for the tenant's instance. The size is judged before
/// Transit is asked for a key, so an oversized body costs no key-service call
/// and a sealed service cannot turn its `413` into a `503`.
pub async fn upload(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    bytes: &[u8],
    now: u64,
) -> Result<UploadedView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    if !granted(&row).contains(&Scope::Spend) {
        return Err(ApiError::scope_not_granted(Scope::Spend));
    }
    let size = u64::try_from(bytes.len()).map_err(|_| ApiError::ATTACHMENT_TOO_LARGE)?;
    envelope::size_verdict(size, api.settings.max_bytes)
        .map_err(|_| ApiError::ATTACHMENT_TOO_LARGE)?;

    let data_key = api.keys.data_key(api.transit_key).await?;
    // Never log the refused value: a service that answered a bare key where a
    // wrapped one belongs would have it written out.
    let wrapped_key = WrappedKey::parse(data_key.wrapped).ok_or_else(|| {
        tracing::error!("the key service answered a data key that is not wrapped");
        ApiError::INTERNAL
    })?;
    let id = Uuid::new_v4();
    let aad = envelope::aad(id, user, row.id);
    let ciphertext = envelope::seal(&data_key.plaintext, &aad, bytes).map_err(seal_failed)?;
    let sealed_digest =
        envelope::seal_digest(&data_key.plaintext, &aad, bytes).map_err(seal_failed)?;
    drop(data_key.plaintext);

    let staged = api
        .attachments
        .staged(
            user,
            NewAttachment {
                id,
                instance_id: row.id,
                ciphertext,
                wrapped_key,
                sealed_digest,
                expires_at: envelope::expiry_for(now, api.settings.ttl_secs),
            },
        )
        .await
        .map_err(store_failed)?;
    Ok(UploadedView {
        attachment_id: staged.id.to_string(),
        size,
        sha256: lowercase_hex(&Sha256::digest(bytes)),
        expires_at: staged.expires_at,
    })
}

pub(super) fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn seal_failed(error: EnvelopeError) -> ApiError {
    tracing::error!(%error, "an attachment could not be sealed");
    ApiError::INTERNAL
}

/// The raw request body, refused under the upload's own codes. axum's
/// rejection answers in prose; the one fault a client can act on, a body
/// past the route's `DefaultBodyLimit`, is the `413` the use-case gives an
/// oversized body, so the two limits cannot be told apart.
pub struct UploadBody(pub Bytes);

impl<S> FromRequest<S> for UploadBody
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        match Bytes::from_request(request, state).await {
            Ok(bytes) => Ok(Self(bytes)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::ATTACHMENT_TOO_LARGE)
            }
            Err(_) => Err(ApiError::INVALID_REQUEST),
        }
    }
}

pub async fn post_upload(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
    UploadBody(bytes): UploadBody,
) -> Result<(StatusCode, Json<UploadedView>), ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    let uploaded = upload(&api, current.user, &instance_id, &bytes, unix_now()).await?;
    Ok((StatusCode::CREATED, Json(uploaded)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::DefaultBodyLimit;
    use axum::handler::{Handler, HandlerWithoutStateExt};
    use std::net::Ipv4Addr;

    #[test]
    fn hex_is_lowercase_and_two_digits_a_byte() {
        assert_eq!(lowercase_hex(&[0x00, 0x0a, 0xff]), "000aff");
    }

    #[test]
    fn the_view_does_not_debug_print_the_digest() {
        let view = UploadedView {
            attachment_id: "id".to_owned(),
            size: 1,
            sha256: "abcdef0123".to_owned(),
            expires_at: 2,
        };
        assert!(!format!("{view:?}").contains("abcdef0123"));
    }

    async fn posted(limit: usize, body: Vec<u8>) -> (u16, String) {
        let handler = (|UploadBody(bytes): UploadBody| async move { bytes.len().to_string() })
            .layer(DefaultBodyLimit::max(limit));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, handler.into_make_service()).await });
        let response = reqwest::Client::new()
            .post(format!("http://{addr}/"))
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.text().await.unwrap())
    }

    #[tokio::test]
    async fn a_body_past_the_route_limit_is_the_upload_s_own_413() {
        assert_eq!(posted(4, vec![0; 4]).await, (200, "4".to_owned()));
        assert_eq!(
            posted(4, vec![0; 5]).await,
            (413, r#"{"error":"attachment_too_large"}"#.to_owned())
        );
    }
}
