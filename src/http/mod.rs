//! Request body parsing and size limits shared by server adapters.

mod body;
mod limited_body;

pub use body::{ParsedJsonBody, parse_json_body, parse_json_body_with_source};
pub use limited_body::{BodySizeLimitError, LimitedBodyCollector};
