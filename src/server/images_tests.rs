use super::*;
use axum::http::{Response as HttpResponse, StatusCode};

fn multipart(fields: &[(&str, &str)], files: &[(&str, &str, &[u8])]) -> (HeaderMap, Vec<u8>) {
    let mut data = Vec::new();
    for (name, value) in fields {
        data.extend_from_slice(format!("--test-boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    for (name, filename, bytes) in files {
        data.extend_from_slice(format!("--test-boundary\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
        data.extend_from_slice(bytes);
        data.extend_from_slice(b"\r\n");
    }
    data.extend_from_slice(b"--test-boundary--\r\n");
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static("multipart/form-data; boundary=test-boundary"),
    );
    (headers, data)
}

async fn response_bytes(response: Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), body::MAX_JSON_BODY_BYTES)
        .await
        .unwrap()
        .to_vec()
}

#[tokio::test]
async fn edit_upload_converts_multiple_images_mask_and_parameters_to_native_json() {
    let (headers, data) = multipart(
        &[
            ("prompt", "change background"),
            ("model", "gpt-image-2.5-flare"),
            ("n", "2"),
            ("size", "1024x1536"),
            ("quality", "high"),
            ("stream", "true"),
            ("response_format", "url"),
            ("output_compression", "70"),
            ("extra", "keep"),
        ],
        &[
            ("image[]", "first.png", b"\x89PNG\x00\xff"),
            ("image", "second.png", b"two"),
            ("mask", "mask.png", b"mask"),
        ],
    );
    // Split the upload across transport chunks, including inside binary file data.
    let chunks: Vec<_> = data
        .chunks(7)
        .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
        .collect();
    let request = prepare_request(
        &headers,
        Body::from_stream(stream::iter(chunks)),
        ImageOperation::Edit,
    )
    .await
    .unwrap();
    assert_eq!(request.response_format, ImageResponseFormat::Url);
    assert_eq!(
        Value::Object(request.body),
        json!({
            "model":"gpt-image-2.5-flare", "prompt":"change background", "n":2,
            "size":"1024x1536", "quality":"high", "stream":true, "output_compression":70, "extra":"keep",
            "images":[{"image_url":format!("data:image/png;base64,{}", STANDARD.encode(b"\x89PNG\x00\xff"))},
                      {"image_url":"data:image/png;base64,dHdv"}],
            "mask":{"image_url":"data:image/png;base64,bWFzaw=="}
        })
    );
}

#[tokio::test]
async fn native_json_edit_keeps_file_ids_and_future_options() {
    let input = json!({"prompt":"edit", "images":[{"file_id":"file-1"}], "mask":{"file_id":"mask-1"}, "future_option":true});
    let request = prepare_request(
        &HeaderMap::new(),
        Body::from(input.to_string()),
        ImageOperation::Edit,
    )
    .await
    .unwrap();
    assert_eq!(request.body["images"], input["images"]);
    assert_eq!(request.body["mask"], input["mask"]);
    assert_eq!(request.body["future_option"], true);
    assert!(!request.body.contains_key("model"));
}

#[tokio::test]
async fn rejects_malformed_missing_and_oversized_uploads() {
    let (mut headers, data) = multipart(&[("prompt", "edit")], &[]);
    let error = prepare_request(&headers, Body::from(data.clone()), ImageOperation::Edit)
        .await
        .unwrap_err();
    assert_eq!(error.param.as_deref(), Some("images"));
    headers.insert(
        "content-length",
        HeaderValue::from_str(&(MAX_IMAGE_MULTIPART_BYTES + 1).to_string()).unwrap(),
    );
    let error = prepare_request(&headers, Body::empty(), ImageOperation::Edit)
        .await
        .unwrap_err();
    assert_eq!(error.status, 413);
    headers.remove("content-length");
    headers.insert(
        "content-type",
        HeaderValue::from_static("multipart/form-data"),
    );
    assert_eq!(
        prepare_request(&headers, Body::from(data), ImageOperation::Edit)
            .await
            .unwrap_err()
            .status,
        400
    );
    headers.insert(
        "content-type",
        HeaderValue::from_static("multipart/form-data; boundary=test-boundary"),
    );
    assert_eq!(
        prepare_request(
            &headers,
            Body::from("broken multipart"),
            ImageOperation::Edit
        )
        .await
        .unwrap_err()
        .status,
        400
    );
}

#[tokio::test]
async fn rejects_empty_files_ambiguous_masks_and_non_edit_uploads() {
    for (fields, files) in [
        (
            vec![("prompt", "edit")],
            vec![("image", "empty.png", b"".as_slice())],
        ),
        (
            vec![("prompt", "edit"), ("mask[file_id]", "mask-1")],
            vec![
                ("image", "a.png", b"a".as_slice()),
                ("mask", "m.png", b"m".as_slice()),
            ],
        ),
    ] {
        let (headers, data) = multipart(&fields, &files);
        assert!(
            prepare_request(&headers, Body::from(data), ImageOperation::Edit)
                .await
                .is_err()
        );
    }
    let (headers, data) = multipart(&[("prompt", "generate")], &[("image", "a.png", b"a")]);
    assert!(
        prepare_request(&headers, Body::from(data), ImageOperation::Generate)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn url_responses_preserve_status_headers_and_usage_but_clear_stale_body_headers() {
    let upstream = reqwest::Response::from(HttpResponse::builder()
        .status(StatusCode::CREATED).header("content-type", "application/json")
        .header("etag", "old").header("content-length", "1").header("x-request-id", "req-1")
        .body(json!({"created":1,"data":[{"b64_json":"AAAA","generation_id":"gen-1"}],"usage":{"total_tokens":10}}).to_string()).unwrap());
    let output = present_response(upstream, ImageResponseFormat::Url, "webp".into())
        .await
        .unwrap();
    assert_eq!(output.status(), StatusCode::CREATED);
    assert_eq!(output.headers()["x-request-id"], "req-1");
    assert!(!output.headers().contains_key("etag"));
    assert!(!output.headers().contains_key("content-length"));
    let payload: Value = serde_json::from_slice(&response_bytes(output).await).unwrap();
    assert_eq!(payload["data"][0]["url"], "data:image/webp;base64,AAAA");
    assert_eq!(payload["data"][0]["generation_id"], "gen-1");
    assert_eq!(payload["usage"]["total_tokens"], 10);
}

#[tokio::test]
async fn base64_stream_and_upstream_errors_are_passed_through_unchanged() {
    for (status, format, content_type, body) in [
        (
            200,
            ImageResponseFormat::Base64,
            "text/event-stream",
            "event: image_generation.completed\ndata: {\"b64_json\":\"AAAA\"}\n\n",
        ),
        (
            429,
            ImageResponseFormat::Url,
            "application/json",
            "{\"error\":{\"message\":\"quota exceeded\"}}",
        ),
    ] {
        let upstream = reqwest::Response::from(
            HttpResponse::builder()
                .status(status)
                .header("content-type", content_type)
                .header("retry-after", "60")
                .body(body)
                .unwrap(),
        );
        let output = present_response(upstream, format, "png".into())
            .await
            .unwrap();
        assert_eq!(output.status().as_u16(), status);
        assert_eq!(output.headers()["retry-after"], "60");
        assert_eq!(response_bytes(output).await, body.as_bytes());
    }
}

#[tokio::test]
async fn url_stream_transforms_each_event_and_reports_bad_frames() {
    for (wire, expected) in [
        (
            "event: image_edit.completed\ndata: {\"type\":\"image_edit.completed\",\"b64_json\":\"AAAA\"}\n\n",
            "data:image/png;base64,AAAA",
        ),
        ("data: {bad}\n\n", "event: error\ndata: {\"error\":"),
    ] {
        let upstream = reqwest::Response::from(
            HttpResponse::builder()
                .header("content-type", "text/event-stream")
                .body(wire)
                .unwrap(),
        );
        let output = present_response(upstream, ImageResponseFormat::Url, "png".into())
            .await
            .unwrap();
        let output = String::from_utf8(response_bytes(output).await).unwrap();
        assert!(output.contains(expected), "{output}");
    }
}
