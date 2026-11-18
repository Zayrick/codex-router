//! OpenAI Images requests adapted to the native Codex Images endpoints.

mod stream;

pub use stream::ImageSseAdapter;

use serde_json::{Value, json};

use crate::core::{ApiError, AppResult, JsonObject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOperation {
    Generate,
    Edit,
}

impl ImageOperation {
    pub const fn path(self) -> &'static str {
        match self {
            Self::Generate => "/v1/images/generations",
            Self::Edit => "/v1/images/edits",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageResponseFormat {
    Base64,
    Url,
}

#[derive(Debug)]
pub struct ImageRequest {
    pub operation: ImageOperation,
    pub body: JsonObject,
    pub response_format: ImageResponseFormat,
    pub output_format: String,
}

pub fn adapt_image_request(
    mut body: JsonObject,
    operation: ImageOperation,
) -> AppResult<ImageRequest> {
    required_string(body.get("prompt"), "prompt")?;
    if let Some(value) = body.get("model")
        && !value.is_null()
    {
        required_string(Some(value), "model")?;
    }
    let response_format = match body.remove("response_format") {
        None | Some(Value::Null) => ImageResponseFormat::Base64,
        Some(Value::String(value)) if value == "b64_json" => ImageResponseFormat::Base64,
        Some(Value::String(value)) if value == "url" => ImageResponseFormat::Url,
        Some(_) => {
            return Err(invalid_image(
                "response_format must be b64_json or url.",
                "response_format",
            ));
        }
    };
    if let Some(value) = body.get("stream")
        && !value.is_null()
        && !value.is_boolean()
    {
        return Err(invalid_image("stream must be a boolean.", "stream"));
    }
    if let Some(value) = body.get("n")
        && !value.is_null()
        && !value.as_u64().is_some_and(|n| n > 0)
    {
        return Err(invalid_image("n must be a positive integer.", "n"));
    }
    for field in ["size", "quality", "background", "output_format"] {
        if let Some(value) = body.get(field)
            && !value.is_null()
        {
            required_string(Some(value), field)?;
        }
    }
    if operation == ImageOperation::Edit {
        let images = body
            .get("images")
            .and_then(Value::as_array)
            .filter(|images| !images.is_empty())
            .ok_or_else(|| invalid_image("At least one input image is required.", "images"))?;
        for (index, image) in images.iter().enumerate() {
            validate_image_reference(image, &format!("images[{index}]"))?;
        }
        if let Some(mask) = body.get("mask")
            && !mask.is_null()
        {
            validate_image_reference(mask, "mask")?;
        }
    }
    let output_format = body
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or("png")
        .to_owned();
    Ok(ImageRequest {
        operation,
        body,
        response_format,
        output_format,
    })
}

/// Preserve native file references as well as inline/remote image URLs without fetching them.
fn validate_image_reference(value: &Value, param: &str) -> AppResult<()> {
    let Some(reference) = value.as_object() else {
        return Err(invalid_image(
            "An image must contain image_url or file_id.",
            param,
        ));
    };
    if ["image_url", "file_id"].iter().any(|field| {
        reference
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    }) {
        Ok(())
    } else {
        Err(invalid_image(
            "An image must contain image_url or file_id.",
            param,
        ))
    }
}

fn required_string<'a>(value: Option<&'a Value>, param: &str) -> AppResult<&'a str> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid_image(format!("{param} must be a non-empty string."), param))
}

pub(crate) fn invalid_image(message: impl Into<String>, param: &str) -> ApiError {
    super::invalid_request(message, "invalid_image_request", Some(param))
}

/// Multipart values are strings on the wire, but native Images expects typed JSON.
pub fn insert_form_field(body: &mut JsonObject, name: &str, value: String) -> AppResult<()> {
    let parsed = match name {
        "n" | "output_compression" | "partial_images" | "frame_count" | "frame_duration_ms" => {
            Value::from(value.trim().parse::<u64>().map_err(|_| {
                invalid_image(format!("{name} must be a non-negative integer."), name)
            })?)
        }
        "stream" => Value::Bool(
            value
                .trim()
                .parse::<bool>()
                .map_err(|_| invalid_image("stream must be true or false.", name))?,
        ),
        "images" | "mask" => serde_json::from_str(&value)
            .map_err(|_| invalid_image(format!("{name} must be valid JSON."), name))?,
        "mask[file_id]" | "mask[image_url]" => {
            let field = if name == "mask[file_id]" {
                "file_id"
            } else {
                "image_url"
            };
            let mask = body.entry("mask").or_insert_with(|| json!({}));
            let mask = mask
                .as_object_mut()
                .ok_or_else(|| invalid_image("mask must be an object.", "mask"))?;
            mask.insert(field.into(), Value::String(value));
            return Ok(());
        }
        _ => Value::String(value),
    };
    body.insert(name.to_owned(), parsed);
    Ok(())
}

/// Convert only image data; keep usage, generation IDs, errors and unknown fields intact.
pub fn image_response_as_urls(value: &mut Value, fallback_format: &str) -> AppResult<()> {
    let root = value.as_object_mut().ok_or_else(invalid_image_response)?;
    if root.contains_key("error") || root.get("type").and_then(Value::as_str) == Some("error") {
        return Ok(());
    }
    let format = root
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or(fallback_format)
        .to_owned();
    if let Some(data) = root.get_mut("data") {
        let items = data.as_array_mut().ok_or_else(invalid_image_response)?;
        for item in items {
            let item = item.as_object_mut().ok_or_else(invalid_image_response)?;
            convert_image_data(item, &format)?;
        }
    } else {
        // Streaming image_generation/image_edit events carry b64_json at the top level.
        convert_image_data(root, &format)?;
    }
    Ok(())
}

fn convert_image_data(object: &mut JsonObject, fallback_format: &str) -> AppResult<()> {
    let Some(data) = object.remove("b64_json") else {
        return Ok(());
    };
    let data = data.as_str().ok_or_else(invalid_image_response)?;
    let format = object
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or(fallback_format);
    let mime = match format {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => return Err(invalid_image_response()),
    };
    object.insert(
        "url".into(),
        Value::String(format!("data:{mime};base64,{data}")),
    );
    Ok(())
}

pub(crate) fn invalid_image_response() -> ApiError {
    ApiError::new(502, "The Codex backend returned an invalid image response.")
        .with_kind("upstream_error")
        .with_code("invalid_image_response")
}

#[cfg(test)]
mod tests;
