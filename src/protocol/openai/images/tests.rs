use super::*;

fn object(value: Value) -> JsonObject {
    value.as_object().unwrap().clone()
}

#[test]
fn defaults_only_the_model_and_retains_native_parameters() {
    let mut input = object(json!({
        "prompt": "a blue circle", "size": "2048x1152", "quality": "low", "n": 2,
        "background": "transparent", "output_format": "webp", "output_compression": 70,
        "partial_images": 2, "stream": true, "extension": {"keep": true}
    }));
    let adapted = adapt_image_request(input.clone(), ImageOperation::Generate).unwrap();
    input.insert("model".into(), json!("gpt-image-2"));
    assert_eq!(adapted.body, input);
    assert_eq!(adapted.response_format, ImageResponseFormat::Base64);
    assert_eq!(adapted.output_format, "webp");
}

#[test]
fn preserves_explicit_models_and_native_image_and_mask_references() {
    let input = object(json!({
        "model": "future-image-model", "prompt": "change the background",
        "images": [{"file_id": "file-1"}, {"image_url": "data:image/png;base64,AA=="}],
        "mask": {"file_id": "mask-1"}, "response_format": "url"
    }));
    let request = adapt_image_request(input.clone(), ImageOperation::Edit).unwrap();
    let mut expected = input;
    expected.remove("response_format");
    assert_eq!(request.body, expected);
    assert_eq!(request.response_format, ImageResponseFormat::Url);
}

#[test]
fn rejects_invalid_requests_before_sending_them_to_the_upstream() {
    for (input, operation, param) in [
        (json!({"prompt":" "}), ImageOperation::Generate, "prompt"),
        (
            json!({"prompt":"test", "model":42}),
            ImageOperation::Generate,
            "model",
        ),
        (
            json!({"prompt":"test", "response_format":"binary"}),
            ImageOperation::Generate,
            "response_format",
        ),
        (
            json!({"prompt":"test", "stream":"true"}),
            ImageOperation::Generate,
            "stream",
        ),
        (
            json!({"prompt":"test", "n":0}),
            ImageOperation::Generate,
            "n",
        ),
        (
            json!({"prompt":"test", "size":1024}),
            ImageOperation::Generate,
            "size",
        ),
        (
            json!({"prompt":"test", "images":[]}),
            ImageOperation::Edit,
            "images",
        ),
        (
            json!({"prompt":"test", "images":[{}]}),
            ImageOperation::Edit,
            "images[0]",
        ),
        (
            json!({"prompt":"test", "images":[{"file_id":"f"}], "mask":"x"}),
            ImageOperation::Edit,
            "mask",
        ),
    ] {
        let error = adapt_image_request(object(input), operation).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.param.as_deref(), Some(param));
    }
}

#[test]
fn multipart_fields_keep_types_and_native_mask_references() {
    let mut body = JsonObject::new();
    for (name, value) in [
        ("n", "2"),
        ("output_compression", "70"),
        ("partial_images", "0"),
        ("stream", "true"),
        ("mask[file_id]", "mask-1"),
        ("quality", "high"),
        ("images", r#"[{"file_id":"file-1"}]"#),
    ] {
        insert_form_field(&mut body, name, value.into()).unwrap();
    }
    assert_eq!(
        Value::Object(body),
        json!({
            "n":2, "output_compression":70, "partial_images":0, "stream":true,
            "mask":{"file_id":"mask-1"}, "quality":"high", "images":[{"file_id":"file-1"}]
        })
    );
    for (name, value) in [
        ("n", "1.5"),
        ("partial_images", "-1"),
        ("stream", "yes"),
        ("images", "bad json"),
    ] {
        assert!(insert_form_field(&mut JsonObject::new(), name, value.into()).is_err());
    }
}

#[test]
fn response_urls_keep_usage_metadata_and_per_image_formats() {
    let mut payload = json!({
        "created": 1, "output_format": "webp", "size": "1024x1024", "quality":"high",
        "usage":{"total_tokens":123}, "extension":true,
        "data":[{"b64_json":"AAAA", "generation_id":"gen-1"},
                {"b64_json":"BBBB", "output_format":"jpeg", "revised_prompt":"hello"}]
    });
    image_response_as_urls(&mut payload, "png").unwrap();
    assert_eq!(
        payload,
        json!({
            "created":1, "output_format":"webp", "size":"1024x1024", "quality":"high",
            "usage":{"total_tokens":123}, "extension":true,
            "data":[{"url":"data:image/webp;base64,AAAA", "generation_id":"gen-1"},
                    {"url":"data:image/jpeg;base64,BBBB", "output_format":"jpeg", "revised_prompt":"hello"}]
        })
    );
    let mut error = json!({"error":{"message":"quota exceeded"}});
    let original = error.clone();
    image_response_as_urls(&mut error, "png").unwrap();
    assert_eq!(error, original);
}

#[test]
fn streaming_conversion_handles_split_utf8_crlf_and_preserves_sse_metadata() {
    let wire = concat!(
        ": keepalive\r\n\r\n",
        "event: image_generation.partial_image\r\nid: image-1\r\nretry: 3000\r\n",
        "data: {\"type\":\"image_generation.partial_image\",\"b64_json\":\"AAAA\",\r\n",
        "data: \"partial_image_index\":0,\"note\":\"你好\"}\r\n\r\n",
        "event: image_generation.completed\ndata: {\"type\":\"image_generation.completed\",\"b64_json\":\"BBBB\",\"usage\":{\"total_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut adapter = ImageSseAdapter::new("jpeg".into());
    let mut output = Vec::new();
    for byte in wire.as_bytes() {
        output.extend(adapter.push_bytes(&[*byte]).unwrap().concat());
    }
    output.extend(adapter.finish().unwrap().concat());
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(": keepalive\r\n\r\n"));
    assert!(output.contains("event: image_generation.partial_image\nid: image-1\nretry: 3000\n"));
    assert!(output.contains("data:image/jpeg;base64,AAAA"));
    assert!(output.contains("data:image/jpeg;base64,BBBB"));
    assert!(output.contains("你好"));
    assert!(output.contains(r#""usage":{"total_tokens":4}"#));
    assert!(!output.contains("b64_json"));
    assert!(output.ends_with("data: [DONE]\n\n"));
}

#[test]
fn streaming_conversion_flushes_at_eof_and_rejects_invalid_frames() {
    let mut adapter = ImageSseAdapter::new("png".into());
    assert!(
        adapter
            .push_bytes(b"data: {\"b64_json\":\"AAAA\"}")
            .unwrap()
            .is_empty()
    );
    let result = String::from_utf8(adapter.finish().unwrap().concat()).unwrap();
    assert!(result.contains("data:image/png;base64,AAAA"));
    assert!(adapter.finish().unwrap().is_empty());
    assert!(adapter.push_bytes(b"data: {invalid}\n\n").is_err());
    let mut adapter = ImageSseAdapter::new("png".into());
    assert!(adapter.push_bytes(b"data: \xff\n\n").is_err());
}

#[test]
fn streaming_images_can_exceed_the_text_event_limit() {
    let data = "A".repeat(super::super::sse::MAX_SSE_EVENT_CHARS + 16);
    let wire = format!("data: {{\"b64_json\":\"{data}\"}}\n\n");
    let mut adapter = ImageSseAdapter::new("png".into());
    let frames = adapter.push_bytes(wire.as_bytes()).unwrap();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].len() > data.len());
    assert!(adapter.finish().unwrap().is_empty());
}
