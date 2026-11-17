use std::convert::Infallible;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};

use crate::{
    core::{AppResult, JsonObject},
    protocol::openai::images::{
        ImageOperation, ImageRequest, ImageResponseFormat, ImageSseAdapter, adapt_image_request,
        image_response_as_urls, insert_form_field, invalid_image, invalid_image_response,
    },
};

use super::{body, response};

// Leaves room for base64 expansion within the existing 100 MiB JSON request budget.
const MAX_IMAGE_MULTIPART_BYTES: usize = 64 * 1024 * 1024;

pub async fn prepare_request(
    headers: &HeaderMap,
    body: Body,
    operation: ImageOperation,
) -> AppResult<ImageRequest> {
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let multipart = content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data"));
    let input = if multipart {
        if operation != ImageOperation::Edit {
            return Err(invalid_image(
                "Image generation requires a JSON body.",
                "body",
            ));
        }
        read_multipart(headers, body, content_type).await?
    } else {
        body::request_json(headers, body).await?
    };
    adapt_image_request(input, operation)
}

async fn read_multipart(
    headers: &HeaderMap,
    body: Body,
    content_type: &str,
) -> AppResult<JsonObject> {
    if headers
        .get("content-encoding")
        .is_some_and(|value| value.as_bytes() != b"identity")
    {
        return Err(invalid_image(
            "Compressed multipart image uploads are not supported.",
            "body",
        ));
    }
    let boundary = multer::parse_boundary(content_type)
        .map_err(|_| invalid_image("The multipart boundary is invalid.", "body"))?;
    let bytes = body::read_limited_body(headers, body, MAX_IMAGE_MULTIPART_BYTES).await?;
    let source = stream::once(async move { Ok::<Bytes, Infallible>(Bytes::from(bytes)) });
    let mut multipart = multer::Multipart::new(source, boundary);
    let mut input = JsonObject::new();
    let mut images = Vec::new();
    let mut mask = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| invalid_image("The multipart image upload is invalid.", "body"))?
    {
        let name = field.name().unwrap_or("").to_owned();
        if name.is_empty() {
            return Err(invalid_image("Multipart fields must have a name.", "body"));
        }
        if field.file_name().is_some() {
            if !matches!(name.as_str(), "image" | "image[]" | "mask") {
                return Err(invalid_image("Unsupported image upload field.", &name));
            }
            let mime = field
                .content_type()
                .map(ToString::to_string)
                .filter(|mime| mime != "application/octet-stream")
                .or_else(|| {
                    field
                        .file_name()
                        .and_then(|name| mime_guess::from_path(name).first_raw().map(str::to_owned))
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            let data = field
                .bytes()
                .await
                .map_err(|_| invalid_image("The uploaded image could not be read.", &name))?;
            if data.is_empty() {
                return Err(invalid_image("The uploaded image is empty.", &name));
            }
            let image =
                json!({"image_url": format!("data:{mime};base64,{}", STANDARD.encode(data))});
            if name == "mask" {
                if mask.replace(image).is_some() {
                    return Err(invalid_image("Only one mask is supported.", "mask"));
                }
            } else {
                images.push(image);
            }
        } else {
            if matches!(name.as_str(), "image" | "image[]") {
                return Err(invalid_image(
                    "image must be an uploaded file; use images for JSON references.",
                    &name,
                ));
            }
            let value = field
                .text()
                .await
                .map_err(|_| invalid_image("The multipart field is invalid.", &name))?;
            insert_form_field(&mut input, &name, value)?;
        }
    }
    if !images.is_empty() {
        input
            .entry("images")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| invalid_image("images must be an array.", "images"))?
            .extend(images);
    }
    if let Some(mask) = mask {
        if input.contains_key("mask") {
            return Err(invalid_image(
                "Specify a mask file or a mask reference, not both.",
                "mask",
            ));
        }
        input.insert("mask".into(), mask);
    }
    Ok(input)
}

pub async fn present_response(
    upstream: reqwest::Response,
    format: ImageResponseFormat,
    output_format: String,
) -> AppResult<Response> {
    // Native Images already returns the standard base64 response and streaming event shapes.
    if !upstream.status().is_success() || format == ImageResponseFormat::Base64 {
        return Ok(response::upstream_proxy(upstream));
    }
    let is_sse = upstream
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    let status = upstream.status();
    let mut headers = response::proxy_headers(upstream.headers());
    for name in [
        "content-length",
        "content-encoding",
        "etag",
        "content-md5",
        "digest",
        "content-digest",
        "repr-digest",
    ] {
        headers.remove(name);
    }
    let body = if is_sse {
        let mut source = upstream.bytes_stream();
        let mut adapter = ImageSseAdapter::new(output_format);
        let output = async_stream::stream! {
            while let Some(chunk) = source.next().await {
                let frames = chunk.map_err(|_| invalid_image_response())
                    .and_then(|bytes| adapter.push_bytes(&bytes));
                match frames {
                    Ok(frames) => for frame in frames {
                        yield Ok::<Bytes, Infallible>(Bytes::from(frame));
                    },
                    Err(error) => {
                        yield Ok(Bytes::from(format!("event: error\ndata: {}\n\n", error.openai_payload())));
                        return;
                    }
                }
            }
            match adapter.finish() {
                Ok(frames) => for frame in frames {
                    yield Ok(Bytes::from(frame));
                },
                Err(error) => yield Ok(Bytes::from(format!("event: error\ndata: {}\n\n", error.openai_payload()))),
            }
        };
        Body::from_stream(output)
    } else {
        let bytes = body::read_limited_response(upstream, body::MAX_JSON_BODY_BYTES).await?;
        let mut payload: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid_image_response())?;
        image_response_as_urls(&mut payload, &output_format)?;
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        Body::from(serde_json::to_vec(&payload).map_err(|_| invalid_image_response())?)
    };
    let mut output = Response::new(body);
    *output.status_mut() = status;
    *output.headers_mut() = headers;
    Ok(output)
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
