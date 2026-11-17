//! Authentication persistence and OAuth domain services.

mod credential_import;
mod credentials;
mod crypto;
mod device_flow;
mod oauth_ports;
mod oauth_provider;
mod record_id;
mod refresh;
mod routing;
mod store;
mod users;

#[cfg(test)]
mod oauth_tests;

pub use credential_import::{CredentialImport, MAX_CREDENTIAL_IMPORT_BYTES};
pub(crate) use credentials::CODEX_ACCOUNT_OAUTH_KEY_PREFIX;
pub use credentials::{
    OAuthRepository, OAuthStatus, StoredOAuthCredentials, credentials_from_token_response,
    oauth_status,
};
pub use crypto::{constant_time_equal, sha256, sign_json, verify_json};
pub use device_flow::{
    DEFAULT_POLL_INTERVAL_SECONDS, DEVICE_LIFETIME_MS, DeviceAuthorization,
    DeviceAuthorizationService, DevicePollResult,
};
pub use oauth_ports::{
    OAuthClock, OAuthCredentialsStore, OAuthHttpClient, OAuthHttpFailure, OAuthHttpRequest,
    OAuthHttpResponse,
};
pub use oauth_provider::{
    DEVICE_VERIFICATION_URL, MAX_OAUTH_RESPONSE_BYTES, OAuthProvider, PROVIDER_REQUEST_TIMEOUT_MS,
    ProviderDeviceAuthorization, ProviderDevicePollResult,
};
pub(crate) use record_id::{derived_record_id, new_record_id, valid_record_id};
pub use refresh::{OAuthRefreshResult, OAuthRefreshService, REFRESH_WINDOW_MS, oauth_refresh_due};
pub use routing::{
    ACCOUNT_ROUTING_KEY, AccountGroup, AccountRoutingState, CodexAccount, FALLBACK_STRATEGY,
    RouteAssignment, RouteTargetKind, RoutingRepository, WEIGHTED_ROUND_ROBIN_STRATEGY,
    session_affinity_ttl,
};
pub(crate) use routing::{
    CURRENT_ROUTING_VERSION, StoredAccountRouting, normalized_account_name, unique_account_name,
};
pub use store::StateStore;
pub use users::{
    AuthenticatedUser, CredentialKind, MAX_CREDENTIALS_PER_USER, USERS_KEY, User, UserCredential,
    UserRepository, client_token, match_account_id, match_api_key,
};
pub(crate) use users::{CURRENT_USERS_VERSION, StoredUsers};
