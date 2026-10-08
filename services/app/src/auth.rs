use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use subtle::ConstantTimeEq;
use url::Url;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    models::User,
    state::AppState,
};

const CREATOR_COOKIE: &str = "interdeck_session";
const OAUTH_COOKIE: &str = "interdeck_oauth";

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    sub: String,
    email: String,
    email_verified: bool,
    name: Option<String>,
    picture: Option<String>,
    hd: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user: User,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoginQuery>,
) -> AppResult<Response> {
    let oauth_state = random_token(32);
    let verifier = random_token(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let return_to = safe_return_to(query.return_to.as_deref());

    sqlx::query(
        "INSERT INTO oauth_states (state_hash, pkce_verifier, return_to, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(hash_token(&oauth_state))
    .bind(&verifier)
    .bind(&return_to)
    .bind(Utc::now() + Duration::minutes(10))
    .execute(&state.db)
    .await?;

    let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(|error| AppError::Internal(error.into()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config.google_client_id)
        .append_pair("redirect_uri", &state.config.oauth_callback_url())
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email profile")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    if !state.config.google_workspace_domain.is_empty() {
        url.query_pairs_mut()
            .append_pair("hd", &state.config.google_workspace_domain);
    }

    let mut response = Redirect::temporary(url.as_str()).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&oauth_cookie(
            &oauth_state,
            state.config.secure_cookies(),
            600,
        ))
        .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

pub async fn callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> AppResult<Response> {
    if let Some(error) = query.error {
        return Err(AppError::BadRequest(format!(
            "Google sign-in was not completed: {error}"
        )));
    }
    let code = query
        .code
        .ok_or_else(|| AppError::BadRequest("Missing OAuth code".to_owned()))?;
    let oauth_state = query
        .state
        .ok_or_else(|| AppError::BadRequest("Missing OAuth state".to_owned()))?;
    if !oauth_browser_matches(&headers, &oauth_state) {
        return Err(AppError::BadRequest(
            "Sign-in must finish in the browser where it started".to_owned(),
        ));
    }

    let state_row = sqlx::query(
        "DELETE FROM oauth_states WHERE state_hash = $1 AND expires_at > now() RETURNING pkce_verifier, return_to",
    )
    .bind(hash_token(&oauth_state))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("OAuth state is invalid or expired".to_owned()))?;

    let verifier: String = state_row.try_get("pkce_verifier")?;
    let return_to: String = state_row.try_get("return_to")?;

    let token = state
        .http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", state.config.google_client_id.as_str()),
            ("client_secret", state.config.google_client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", state.config.oauth_callback_url().as_str()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<GoogleTokenResponse>()
        .await?;

    let profile = state
        .http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&token.access_token)
        .send()
        .await?
        .error_for_status()?
        .json::<GoogleUserInfo>()
        .await?;

    if !profile.email_verified
        || !state
            .config
            .google_account_allowed(&profile.email, profile.hd.as_deref())
    {
        return Err(AppError::Forbidden);
    }

    let user_id = Uuid::now_v7();
    let display_name = profile.name.unwrap_or_else(|| profile.email.clone());
    let user = sqlx::query(
        r#"
        INSERT INTO users (id, google_sub, email, display_name, picture_url)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (google_sub) DO UPDATE
        SET email = EXCLUDED.email,
            display_name = EXCLUDED.display_name,
            picture_url = EXCLUDED.picture_url,
            updated_at = now()
        RETURNING id, email, display_name, picture_url
        "#,
    )
    .bind(user_id)
    .bind(profile.sub)
    .bind(profile.email.to_lowercase())
    .bind(display_name)
    .bind(profile.picture)
    .fetch_one(&state.db)
    .await?;

    let user = User {
        id: user.try_get("id")?,
        email: user.try_get("email")?,
        display_name: user.try_get("display_name")?,
        picture_url: user.try_get("picture_url")?,
    };

    let session_token = random_token(48);
    sqlx::query("INSERT INTO auth_sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(hash_token(&session_token))
        .bind(user.id)
        .bind(Utc::now() + Duration::days(7))
        .execute(&state.db)
        .await?;

    let mut response = Redirect::to(&safe_return_to(Some(&return_to))).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&oauth_cookie("", state.config.secure_cookies(), 0))
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&creator_cookie(
            &session_token,
            state.config.secure_cookies(),
            7 * 24 * 60 * 60,
        ))
        .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

pub async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<MeResponse>> {
    Ok(Json(MeResponse {
        user: require_user(&state, &headers).await?,
    }))
}

pub async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(token) = cookie_value(&headers, CREATOR_COOKIE) {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = $1")
            .bind(hash_token(token))
            .execute(&state.db)
            .await?;
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("interdeck_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    Ok(response)
}

pub async fn require_user(state: &AppState, headers: &HeaderMap) -> AppResult<User> {
    let token = cookie_value(headers, CREATOR_COOKIE).ok_or(AppError::Unauthorized)?;
    let row = sqlx::query(
        r#"
        SELECT u.id, u.email, u.display_name, u.picture_url
        FROM auth_sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1 AND s.expires_at > now()
        "#,
    )
    .bind(hash_token(token))
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    Ok(User {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        picture_url: row.try_get("picture_url")?,
    })
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

pub fn random_token(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::rng().fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

pub fn hash_token(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

fn safe_return_to(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "/".to_owned();
    };
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return "/".to_owned();
    }
    let base = Url::parse("https://interdeck.invalid/").expect("fixed URL");
    match base.join(value) {
        Ok(url) if url.origin() == base.origin() => value.to_owned(),
        _ => "/".to_owned(),
    }
}

fn oauth_browser_matches(headers: &HeaderMap, state: &str) -> bool {
    cookie_value(headers, OAUTH_COOKIE).is_some_and(|cookie| {
        bool::from(Sha256::digest(cookie.as_bytes()).ct_eq(&Sha256::digest(state.as_bytes())))
    })
}

fn oauth_cookie(token: &str, secure: bool, max_age: i64) -> String {
    format!(
        "{OAUTH_COOKIE}={token}; Path=/api/auth/google; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn creator_cookie(token: &str, secure: bool, max_age: i64) -> String {
    format!(
        "{CREATOR_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_external_and_ambiguous_return_locations() {
        for value in [
            "//evil.example",
            "/\\evil.example",
            "https://evil.example",
            "/\n/evil.example",
            "\\evil.example",
        ] {
            assert_eq!(safe_return_to(Some(value)), "/");
        }
        assert_eq!(
            safe_return_to(Some("/decks/123?tab=edit#source")),
            "/decks/123?tab=edit#source"
        );
    }

    #[test]
    fn callback_requires_the_initiating_browser_cookie() {
        let mut headers = HeaderMap::new();
        assert!(!oauth_browser_matches(&headers, "random-state"));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("interdeck_oauth=another-state"),
        );
        assert!(!oauth_browser_matches(&headers, "random-state"));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("interdeck_oauth=random-state"),
        );
        assert!(oauth_browser_matches(&headers, "random-state"));
        assert!(
            oauth_cookie("state", true, 600)
                .contains("HttpOnly; SameSite=Lax; Max-Age=600; Secure")
        );
    }
}
