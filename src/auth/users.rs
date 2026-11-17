//! Downstream users and the credentials they sign requests with.
//!
//! A user owns any number of API keys and ChatGPT account ids. Requests and
//! console sessions authenticated by any of them are attributed to the user.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::core::{ApiError, AppResult};

use super::{StateStore, new_record_id, sha256, valid_record_id};

pub const USERS_KEY: &str = "USERS";
pub(crate) const CURRENT_USERS_VERSION: u8 = 1;
const MAX_USERS: usize = 100;
pub const MAX_CREDENTIALS_PER_USER: usize = 20;
const MAX_NAME_LENGTH: usize = 100;
const MIN_API_KEY_LENGTH: usize = 11;
const MAX_API_KEY_LENGTH: usize = 512;
const MAX_ACCOUNT_ID_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// Sent as a bearer token, `x-api-key`, or `x-goog-api-key`.
    ApiKey,
    /// The `chatgpt-account-id` of a client relaying its own ChatGPT sign-in.
    AccountId,
}

impl CredentialKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AccountId => "account_id",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCredential {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: CredentialKind,
    pub value: String,
    /// Optional label, e.g. the device using it.
    #[serde(default)]
    pub name: String,
    pub enabled: bool,
}

impl UserCredential {
    /// Label shown in logs: the configured name, else the last four characters.
    pub fn display_name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        let tail: String = {
            let mut tail = self.value.chars().rev().take(4).collect::<Vec<_>>();
            tail.reverse();
            tail.into_iter().collect()
        };
        format!("···{tail}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub credentials: Vec<UserCredential>,
}

impl User {
    pub fn credential(&self, id: &str) -> Option<&UserCredential> {
        self.credentials
            .iter()
            .find(|credential| credential.id == id)
    }

    /// Credentials currently accepted for this user.
    pub fn active_credentials(&self) -> impl Iterator<Item = &UserCredential> {
        self.credentials
            .iter()
            .filter(move |credential| self.enabled && credential.enabled)
    }
}

/// A user together with the credential that authenticated a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user: User,
    pub credential: UserCredential,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoredUsers {
    pub version: u8,
    #[serde(default)]
    pub users: Vec<User>,
}

pub struct UserRepository<'a> {
    store: &'a dyn StateStore,
}

impl<'a> UserRepository<'a> {
    pub fn new(store: &'a dyn StateStore) -> Self {
        Self { store }
    }

    pub async fn read(&self) -> AppResult<Vec<User>> {
        let Some(serialized) = self.store.get(USERS_KEY).await? else {
            return Ok(Vec::new());
        };
        let stored: StoredUsers =
            serde_json::from_str(&serialized).map_err(|_| invalid_stored_users())?;
        if stored.version != CURRENT_USERS_VERSION {
            return Err(invalid_stored_users());
        }
        validate_users(stored.users).map_err(|_| invalid_stored_users())
    }

    async fn store(&self, users: Vec<User>) -> AppResult<Vec<User>> {
        let users = validate_users(users)?;
        let serialized = serde_json::to_string(&StoredUsers {
            version: CURRENT_USERS_VERSION,
            users: users.clone(),
        })
        .map_err(|_| invalid_stored_users())?;
        self.store.put(USERS_KEY, &serialized).await?;
        Ok(users)
    }

    pub async fn user(&self, id: &str) -> AppResult<Option<User>> {
        Ok(self.read().await?.into_iter().find(|user| user.id == id))
    }

    pub async fn create(&self, value: &Value) -> AppResult<Vec<User>> {
        let candidate = parse_user(value, new_record_id(), &[])?;
        let mut users = self.read().await?;
        if users.len() >= MAX_USERS {
            return Err(user_conflict("The user limit has been reached."));
        }
        users.push(candidate);
        self.store(users).await
    }

    pub async fn update(&self, id: &Value, value: &Value) -> AppResult<Vec<User>> {
        let id = user_id(id)?;
        let mut users = self.read().await?;
        let index = users
            .iter()
            .position(|user| user.id == id)
            .ok_or_else(user_not_found)?;
        users[index] = parse_user(value, id, &users[index].credentials)?;
        self.store(users).await
    }

    pub async fn delete(&self, id: &Value) -> AppResult<Vec<User>> {
        let id = user_id(id)?;
        let mut users = self.read().await?;
        let before = users.len();
        users.retain(|user| user.id != id);
        if users.len() == before {
            return Err(user_not_found());
        }
        self.store(users).await
    }

    /// The enabled user and credential behind an API key.
    pub async fn authenticate_api_key(&self, token: Option<&str>) -> AppResult<AuthenticatedUser> {
        let users = self.read().await?;
        match_api_key(token, &users).ok_or_else(invalid_api_key)
    }

    /// The enabled user and credential behind a relayed ChatGPT account id.
    pub async fn match_account_id(
        &self,
        account_id: Option<&str>,
    ) -> AppResult<Option<AuthenticatedUser>> {
        let users = self.read().await?;
        Ok(match_account_id(account_id, &users))
    }
}

/// Compares against every API key so timing does not reveal which one matched.
pub fn match_api_key(token: Option<&str>, users: &[User]) -> Option<AuthenticatedUser> {
    let token = token.filter(|token| utf16_len(token) <= MAX_API_KEY_LENGTH)?;
    let digest = sha256(token);
    let mut matched = None;
    for user in users {
        for credential in user.active_credentials() {
            let equal = credential.kind == CredentialKind::ApiKey
                && bool::from(digest.ct_eq(&sha256(&credential.value)));
            if equal && matched.is_none() {
                matched = Some(AuthenticatedUser {
                    user: user.clone(),
                    credential: credential.clone(),
                });
            }
        }
    }
    matched
}

pub fn match_account_id(account_id: Option<&str>, users: &[User]) -> Option<AuthenticatedUser> {
    let account_id = account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    users.iter().find_map(|user| {
        user.active_credentials()
            .find(|credential| {
                credential.kind == CredentialKind::AccountId && credential.value == account_id
            })
            .map(|credential| AuthenticatedUser {
                user: user.clone(),
                credential: credential.clone(),
            })
    })
}

/// Select a client credential with the same precedence as the public API.
pub fn client_token<'a>(
    authorization: Option<&'a str>,
    api_key: Option<&'a str>,
    google_api_key: Option<&'a str>,
) -> Option<String> {
    bearer_token(authorization)
        .map(str::to_owned)
        .or_else(|| {
            api_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            google_api_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

/// Parses a user from the management API. Credential ids of `existing`
/// credentials are kept; anything else receives a new id.
fn parse_user(value: &Value, id: String, existing: &[UserCredential]) -> AppResult<User> {
    let object = value.as_object().ok_or_else(invalid_user)?;
    let name =
        normalized_name(object.get("name").and_then(Value::as_str)).ok_or_else(invalid_user)?;
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(invalid_user)?;
    let credentials = object
        .get("credentials")
        .and_then(Value::as_array)
        .ok_or_else(invalid_user)?
        .iter()
        .map(|value| parse_credential(value, existing))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(User {
        id,
        name,
        enabled,
        credentials,
    })
}

fn parse_credential(value: &Value, existing: &[UserCredential]) -> AppResult<UserCredential> {
    let object = value.as_object().ok_or_else(invalid_credential)?;
    let kind: CredentialKind = object
        .get("type")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(invalid_credential)?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| existing.iter().any(|credential| credential.id == *id))
        .map_or_else(new_record_id, str::to_owned);
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let name = if name.is_empty() {
        String::new()
    } else {
        normalized_name(Some(name)).ok_or_else(invalid_credential)?
    };
    let credential = UserCredential {
        id,
        kind,
        value: object
            .get("value")
            .and_then(Value::as_str)
            .map(|value| normalized_value(kind, value))
            .ok_or_else(invalid_credential)?,
        name,
        enabled: object
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(invalid_credential)?,
    };
    if !valid_credential_value(credential.kind, &credential.value) {
        return Err(invalid_credential());
    }
    Ok(credential)
}

fn normalized_value(kind: CredentialKind, value: &str) -> String {
    match kind {
        CredentialKind::ApiKey => value.to_owned(),
        CredentialKind::AccountId => value.trim().to_owned(),
    }
}

fn valid_credential_value(kind: CredentialKind, value: &str) -> bool {
    match kind {
        CredentialKind::ApiKey => {
            let len = utf16_len(value);
            (MIN_API_KEY_LENGTH..=MAX_API_KEY_LENGTH).contains(&len)
                && value.bytes().any(|byte| byte.is_ascii_alphabetic())
                && value.bytes().any(|byte| byte.is_ascii_digit())
                && value.chars().any(|character| {
                    !character.is_ascii_alphanumeric() && !character.is_whitespace()
                })
        }
        CredentialKind::AccountId => {
            !value.is_empty()
                && value.len() <= MAX_ACCOUNT_ID_LENGTH
                && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        }
    }
}

/// Checks the whole collection: unique ids and names, and every credential
/// value unique across all users and kinds so a value names one user.
fn validate_users(mut users: Vec<User>) -> AppResult<Vec<User>> {
    if users.len() > MAX_USERS {
        return Err(user_conflict("The user limit has been reached."));
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut credential_ids = HashSet::new();
    let mut values = HashSet::new();
    for user in &mut users {
        if !valid_record_id(&user.id) || !ids.insert(user.id.clone()) {
            return Err(invalid_user());
        }
        user.name = normalized_name(Some(&user.name)).ok_or_else(invalid_user)?;
        if !names.insert(user.name.clone()) {
            return Err(user_conflict("A user with that name already exists."));
        }
        if user.credentials.is_empty() || user.credentials.len() > MAX_CREDENTIALS_PER_USER {
            return Err(invalid_user());
        }
        for credential in &user.credentials {
            if !valid_record_id(&credential.id)
                || !credential_ids.insert(credential.id.clone())
                || !valid_credential_value(credential.kind, &credential.value)
            {
                return Err(invalid_credential());
            }
            if !values.insert(credential.value.clone()) {
                return Err(user_conflict("That credential is already configured."));
            }
        }
    }
    users.sort_by(|left, right| left.name.encode_utf16().cmp(right.name.encode_utf16()));
    Ok(users)
}

fn user_id(value: &Value) -> AppResult<String> {
    value
        .as_str()
        .filter(|value| valid_record_id(value))
        .map(str::to_owned)
        .ok_or_else(invalid_user)
}

fn normalized_name(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty()
        || utf16_len(value) > MAX_NAME_LENGTH
        || value
            .chars()
            .any(|character| matches!(character as u32, 0x00..=0x1f | 0x7f))
    {
        return None;
    }
    Some(value.to_owned())
}

fn bearer_token(authorization: Option<&str>) -> Option<&str> {
    let authorization = authorization?;
    if authorization.starts_with(char::is_whitespace) {
        return None;
    }
    let mut fields = authorization.split_whitespace();
    let scheme = fields.next()?;
    let token = fields.next()?;
    (scheme.eq_ignore_ascii_case("bearer") && fields.next().is_none()).then_some(token)
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn invalid_api_key() -> ApiError {
    ApiError::new(401, "Invalid API key.")
        .with_kind("authentication_error")
        .with_code("invalid_api_key")
}

fn invalid_user() -> ApiError {
    ApiError::new(
        400,
        "Users require a unique name, an enabled state, and 1 to 20 credentials.",
    )
    .with_kind("invalid_request_error")
    .with_code("invalid_user")
}

fn invalid_credential() -> ApiError {
    ApiError::new(400, "API keys need 11 to 512 characters with at least one letter, number, and non-whitespace symbol; account IDs need 1 to 256 visible ASCII characters.")
        .with_kind("invalid_request_error")
        .with_code("invalid_credential")
}

fn user_conflict(message: &str) -> ApiError {
    ApiError::new(409, message)
        .with_kind("invalid_request_error")
        .with_code("user_conflict")
}

fn user_not_found() -> ApiError {
    ApiError::new(404, "The requested user does not exist.")
        .with_kind("invalid_request_error")
        .with_code("user_not_found")
}

fn invalid_stored_users() -> ApiError {
    ApiError::new(500, "Stored users are unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_stored_users")
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<String, String>>,
    }

    #[async_trait]
    impl StateStore for MemoryStore {
        async fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        async fn put(&self, key: &str, value: &str) -> AppResult<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        async fn delete(&self, key: &str) -> AppResult<()> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }

    fn user_input(name: &str, credentials: Value) -> Value {
        json!({ "name": name, "enabled": true, "credentials": credentials })
    }

    #[tokio::test]
    async fn every_credential_of_a_user_authenticates_as_that_user() {
        let store = MemoryStore::default();
        let repository = UserRepository::new(&store);
        let users = repository
            .create(&user_input(
                "alice",
                json!([
                    { "type": "api_key", "value": "sk-alice-laptop-1!", "name": "laptop", "enabled": true },
                    { "type": "api_key", "value": "sk-alice-server-2!", "enabled": true },
                    { "type": "account_id", "value": " account-alice ", "enabled": true },
                ]),
            ))
            .await
            .unwrap();
        let alice = &users[0];
        assert_eq!(alice.credentials[2].value, "account-alice");

        let laptop = match_api_key(Some("sk-alice-laptop-1!"), &users).unwrap();
        let server = match_api_key(Some("sk-alice-server-2!"), &users).unwrap();
        let relay = match_account_id(Some("account-alice"), &users).unwrap();
        assert_eq!(laptop.user.id, alice.id);
        assert_eq!(server.user.id, alice.id);
        assert_eq!(relay.user.id, alice.id);
        assert_ne!(laptop.credential.id, server.credential.id);
        // An account id is not accepted as an API key and vice versa.
        assert!(match_api_key(Some("account-alice"), &users).is_none());
        assert!(match_account_id(Some("sk-alice-laptop-1!"), &users).is_none());
    }

    #[tokio::test]
    async fn disabled_users_and_credentials_are_rejected() {
        let store = MemoryStore::default();
        let repository = UserRepository::new(&store);
        let users = repository
            .create(&user_input(
                "bob",
                json!([
                    { "type": "api_key", "value": "sk-bob-enabled-1!", "enabled": true },
                    { "type": "api_key", "value": "sk-bob-disabled-2!", "enabled": false },
                ]),
            ))
            .await
            .unwrap();
        assert!(match_api_key(Some("sk-bob-enabled-1!"), &users).is_some());
        assert!(match_api_key(Some("sk-bob-disabled-2!"), &users).is_none());

        let id = users[0].id.clone();
        let mut input = user_input("bob", serde_json::to_value(&users[0].credentials).unwrap());
        input["enabled"] = json!(false);
        let users = repository.update(&json!(id), &input).await.unwrap();
        assert!(match_api_key(Some("sk-bob-enabled-1!"), &users).is_none());
    }

    #[tokio::test]
    async fn updates_keep_known_credential_ids_and_assign_new_ones() {
        let store = MemoryStore::default();
        let repository = UserRepository::new(&store);
        let users = repository
            .create(&user_input(
                "carol",
                json!([{ "type": "api_key", "value": "sk-carol-first-1!", "enabled": true }]),
            ))
            .await
            .unwrap();
        let user = &users[0];
        let kept = user.credentials[0].id.clone();
        let forged = "00000000-0000-4000-8000-000000000001";
        let users = repository
            .update(
                &json!(user.id),
                &user_input(
                    "carol",
                    json!([
                        { "id": kept, "type": "api_key", "value": "sk-carol-first-1!", "enabled": true },
                        { "id": forged, "type": "account_id", "value": "account-carol", "enabled": true },
                    ]),
                ),
            )
            .await
            .unwrap();
        assert_eq!(users[0].credentials[0].id, kept);
        assert_ne!(users[0].credentials[1].id, forged);
        assert!(valid_record_id(&users[0].credentials[1].id));
    }

    #[tokio::test]
    async fn credential_values_are_unique_across_users_and_kinds() {
        let store = MemoryStore::default();
        let repository = UserRepository::new(&store);
        repository
            .create(&user_input(
                "dave",
                json!([{ "type": "api_key", "value": "sk-shared-value-1!", "enabled": true }]),
            ))
            .await
            .unwrap();
        let conflict = repository
            .create(&user_input(
                "erin",
                json!([{ "type": "account_id", "value": "sk-shared-value-1!", "enabled": true }]),
            ))
            .await
            .unwrap_err();
        assert_eq!(conflict.status, 409);
        let duplicate_name = repository
            .create(&user_input(
                "dave",
                json!([{ "type": "api_key", "value": "sk-other-value-2!", "enabled": true }]),
            ))
            .await
            .unwrap_err();
        assert_eq!(duplicate_name.status, 409);
    }

    #[test]
    fn validates_credential_formats() {
        assert!(valid_credential_value(
            CredentialKind::ApiKey,
            "A1!aaaaaaaa"
        ));
        assert!(!valid_credential_value(
            CredentialKind::ApiKey,
            "A1!aaaaaaa"
        ));
        assert!(!valid_credential_value(
            CredentialKind::ApiKey,
            "no-digits-here!"
        ));
        assert!(valid_credential_value(
            CredentialKind::AccountId,
            "account-1"
        ));
        assert!(!valid_credential_value(
            CredentialKind::AccountId,
            "line\nbreak"
        ));
        assert!(parse_user(&user_input("empty", json!([])), new_record_id(), &[]).is_ok());
        assert!(
            validate_users(vec![
                parse_user(&user_input("empty", json!([])), new_record_id(), &[]).unwrap()
            ])
            .is_err()
        );
    }

    #[test]
    fn preserves_header_precedence() {
        assert_eq!(
            client_token(Some("Bearer wrong"), Some("right"), Some("google")).as_deref(),
            Some("wrong")
        );
        assert_eq!(
            client_token(None, Some(" right "), Some("google")).as_deref(),
            Some("right")
        );
        assert_eq!(
            client_token(Some("Basic value"), None, Some(" google ")).as_deref(),
            Some("google")
        );
    }

    #[test]
    fn display_names_fall_back_to_the_value_tail() {
        let credential = UserCredential {
            id: new_record_id(),
            kind: CredentialKind::ApiKey,
            value: "sk-display-abcd".into(),
            name: String::new(),
            enabled: true,
        };
        assert_eq!(credential.display_name(), "···abcd");
    }
}
