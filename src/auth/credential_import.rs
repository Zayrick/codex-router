//! Normalize manually supplied OAuth credentials without starting a login flow.

use serde::Deserialize;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::core::{ApiError, AppResult};

use super::{
    OAuthClock, OAuthProvider, StoredOAuthCredentials,
    credentials::{jwt_account_id, jwt_expiry},
    credentials_from_token_response,
};

pub const MAX_CREDENTIAL_IMPORT_BYTES: usize = 64 * 1024;
const MAX_TOKEN_LENGTH: usize = 16 * 1024;

#[derive(Deserialize)]
struct ImportTokens {
    #[serde(alias = "accessToken")]
    access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(alias = "idToken")]
    id_token: Option<String>,
    #[serde(alias = "accountId")]
    account_id: Option<String>,
    email: Option<String>,
    #[serde(alias = "expiresAt")]
    expires_at: Option<Value>,
    #[serde(alias = "expiresIn")]
    expires_in: Option<f64>,
}

pub struct CredentialImport {
    pub name: Option<String>,
    tokens: ImportTokens,
}

impl CredentialImport {
    pub fn parse(value: &Value) -> AppResult<Self> {
        let root = value.as_object().ok_or_else(invalid_import_json)?;
        let source = root.get("credentials").unwrap_or(value);
        let source = source.as_object().ok_or_else(invalid_import_json)?;
        let name = match root.get("name").or_else(|| source.get("name")) {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => text(Some(name.clone()), "name", 100)?,
            _ => return Err(invalid_field("name", "显示名称必须为文本。")),
        };
        let source = match source.get("tokens") {
            Some(tokens) => tokens.as_object().ok_or_else(invalid_import_json)?,
            None => source,
        };
        let mut tokens: ImportTokens = serde_json::from_value(Value::Object(source.clone()))
            .map_err(|_| invalid_import_json())?;
        tokens.access_token = token(tokens.access_token, "access_token", MAX_TOKEN_LENGTH)?;
        tokens.refresh_token = token(tokens.refresh_token, "refresh_token", MAX_TOKEN_LENGTH)?;
        tokens.id_token = token(tokens.id_token, "id_token", MAX_TOKEN_LENGTH)?;
        tokens.account_id = token(tokens.account_id, "account_id", 256)?;
        tokens.email = text(tokens.email, "email", 254)?;
        if tokens.refresh_token.is_none() {
            return Err(invalid_field(
                "refresh_token",
                "请提供 Refresh Token，用于导入凭据和自动续期。",
            ));
        }
        if tokens.expires_in.is_some_and(|seconds| {
            !seconds.is_finite() || seconds <= 0.0 || seconds > i32::MAX as f64
        }) {
            return Err(invalid_field(
                "expires_in",
                "expires_in 必须为有效的正数秒数。",
            ));
        }
        if let Some(expiry) = tokens.expires_at.as_ref() {
            parse_expiry(expiry)?;
        }
        Ok(Self { name, tokens })
    }

    pub fn matches_credentials(&self, stored: &StoredOAuthCredentials) -> bool {
        if self.tokens.refresh_token.as_deref() == Some(stored.refresh_token.as_str()) {
            return true;
        }
        let account_id = self
            .tokens
            .account_id
            .clone()
            .or_else(|| self.tokens.id_token.as_deref().and_then(jwt_account_id))
            .or_else(|| self.tokens.access_token.as_deref().and_then(jwt_account_id));
        account_id.is_some() && account_id == stored.account_id
    }

    pub async fn resolve(
        &self,
        provider: &OAuthProvider<'_>,
        clock: &dyn OAuthClock,
    ) -> AppResult<StoredOAuthCredentials> {
        let now_ms = clock.now_ms().await;
        let expires_at = self
            .tokens
            .access_token
            .as_deref()
            .and_then(jwt_expiry)
            .into_iter()
            .chain(
                self.tokens
                    .expires_at
                    .as_ref()
                    .map(parse_expiry)
                    .transpose()?,
            )
            .chain(
                self.tokens
                    .expires_in
                    .map(|seconds| now_ms.saturating_add((seconds * 1_000.0) as i64)),
            )
            .min();
        let payload = json!({
            "access_token": self.tokens.access_token,
            "refresh_token": self.tokens.refresh_token,
            "id_token": self.tokens.id_token,
            "account_id": self.tokens.account_id,
            "email": self.tokens.email,
        });
        let previous = self
            .tokens
            .access_token
            .as_ref()
            .map(|_| credentials_from_token_response(&payload, None, now_ms))
            .transpose()?;
        let mut credentials = if let (Some(current), Some(expiry)) = (&previous, expires_at)
            && expiry > now_ms
        {
            let mut credentials = current.clone();
            credentials.expires_at = expiry;
            credentials
        } else {
            let mut refreshed = provider
                .refresh_provider_token(
                    self.tokens
                        .refresh_token
                        .as_deref()
                        .expect("validated token"),
                )
                .await?;
            if previous.is_none()
                && let Some(refreshed) = refreshed.as_object_mut()
            {
                for field in ["refresh_token", "id_token", "account_id", "email"] {
                    if refreshed.get(field).is_none_or(|value| {
                        value.as_str().is_none_or(|value| value.trim().is_empty())
                    }) {
                        refreshed.insert(field.into(), payload[field].clone());
                    }
                }
            }
            credentials_from_token_response(&refreshed, previous.as_ref(), clock.now_ms().await)?
        };
        if credentials.expires_at <= clock.now_ms().await {
            return Err(invalid_field(
                "access_token",
                "Access Token 已过期，请提供有效的凭据。",
            ));
        }
        credentials.account_id = token(credentials.account_id, "account_id", 256)?;
        if credentials.account_id.is_none() {
            return Err(invalid_field(
                "account_id",
                "无法从 Token 识别 Account ID，请手动填写 account_id。",
            ));
        }
        Ok(credentials)
    }
}

fn token(value: Option<String>, field: &str, max_length: usize) -> AppResult<Option<String>> {
    let value = text(value, field, max_length)?;
    if value
        .as_ref()
        .is_some_and(|value| !value.bytes().all(|byte| byte.is_ascii_graphic()))
    {
        return Err(invalid_field(
            field,
            "凭据不能包含空格、换行或非 ASCII 字符。",
        ));
    }
    Ok(value)
}

fn text(value: Option<String>, field: &str, max_length: usize) -> AppResult<Option<String>> {
    let value = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| {
        value.encode_utf16().count() > max_length || value.chars().any(char::is_control)
    }) {
        return Err(invalid_field(field, "字段过长或包含控制字符。"));
    }
    Ok(value)
}

fn parse_expiry(value: &Value) -> AppResult<i64> {
    let timestamp = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()));
    let timestamp = match timestamp {
        Some(value) if value > 0 => {
            if value < 1_000_000_000_000 {
                value.checked_mul(1_000)
            } else {
                Some(value)
            }
        }
        None => value.as_str().and_then(|value| {
            OffsetDateTime::parse(value, &Rfc3339)
                .ok()
                .and_then(|value| i64::try_from(value.unix_timestamp_nanos() / 1_000_000).ok())
        }),
        _ => None,
    };
    timestamp
        .filter(|value| *value > 0 && *value <= 253_402_300_799_999)
        .ok_or_else(|| {
            invalid_field(
                "expires_at",
                "expires_at 必须为 Unix 秒/毫秒时间戳或 RFC 3339 时间。",
            )
        })
}

fn invalid_import_json() -> ApiError {
    invalid_field(
        "credentials",
        "凭据格式无效，请提供包含 Token 字段的 JSON 对象或 auth.json。",
    )
}

fn invalid_field(field: &str, message: &str) -> ApiError {
    ApiError::new(400, message)
        .with_kind("invalid_request_error")
        .with_code("invalid_credential_import")
        .with_param(field)
}
