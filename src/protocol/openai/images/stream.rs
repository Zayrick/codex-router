use crate::core::AppResult;

use super::{image_response_as_urls, invalid_image_response};

/// Image events may contain a full base64 image, substantially larger than text SSE events.
const MAX_IMAGE_EVENT_BYTES: usize = 100 * 1024 * 1024;

pub struct ImageSseAdapter {
    line: Vec<u8>,
    event: Vec<u8>,
    output_format: String,
}

impl ImageSseAdapter {
    pub fn new(output_format: String) -> Self {
        Self {
            line: Vec::new(),
            event: Vec::new(),
            output_format,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> AppResult<Vec<Vec<u8>>> {
        let mut output = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if part.len() > MAX_IMAGE_EVENT_BYTES.saturating_sub(self.line.len() + self.event.len())
            {
                return Err(invalid_image_response());
            }
            self.line.extend_from_slice(part);
            if part.last() == Some(&b'\n') {
                let end = matches!(self.line.as_slice(), b"\n" | b"\r\n");
                self.event.append(&mut self.line);
                if end {
                    output.push(self.transform_event()?);
                    self.event.clear();
                }
            }
        }
        Ok(output)
    }

    pub fn finish(&mut self) -> AppResult<Vec<Vec<u8>>> {
        self.event.append(&mut self.line);
        if self.event.is_empty() {
            return Ok(Vec::new());
        }
        let output = self.transform_event()?;
        self.event.clear();
        Ok(vec![output])
    }

    fn transform_event(&self) -> AppResult<Vec<u8>> {
        let event = std::str::from_utf8(&self.event).map_err(|_| invalid_image_response())?;
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|value| value.strip_prefix(' ').unwrap_or(value))
            .collect::<Vec<_>>()
            .join("\n");
        if data.trim().is_empty() || data.trim() == "[DONE]" {
            return Ok(self.event.clone());
        }
        let mut value = serde_json::from_str(&data).map_err(|_| invalid_image_response())?;
        image_response_as_urls(&mut value, &self.output_format)?;
        let mut output = String::new();
        let mut wrote_data = false;
        for line in event.lines().filter(|line| !line.is_empty()) {
            if line.starts_with("data:") {
                if !wrote_data {
                    output.push_str(&format!("data: {value}\n"));
                    wrote_data = true;
                }
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }
        output.push('\n');
        Ok(output.into_bytes())
    }
}
