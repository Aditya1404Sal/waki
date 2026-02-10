mod constants;
pub(crate) mod parser;

use crate::header::{HeaderMap, HeaderValue, IntoHeaderName, CONTENT_DISPOSITION, CONTENT_TYPE};

use anyhow::{Error, Result};
use mime::Mime;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct Form {
    parts: Vec<Part>,
    boundary: String,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    pub fn new() -> Self {
        Self {
            parts: vec![],
            boundary: format!("--FormBoundary{}", generate_random_string(10)),
        }
    }

    pub(crate) fn boundary(&self) -> &str {
        &self.boundary
    }

    pub fn text<S, V>(mut self, key: S, value: V) -> Self
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
    {
        self.parts.push(Part::new(key, value));
        self
    }

    pub fn file<S, P>(mut self, key: S, path: P) -> Result<Self>
    where
        S: Into<String>,
        P: AsRef<Path>,
    {
        self.parts.push(Part::file(key, path)?);
        Ok(self)
    }

    pub fn part(mut self, part: Part) -> Self {
        self.parts.push(part);
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut buf = vec![];
        for part in self.parts {
            buf.extend_from_slice(
                format!(
                    "{}{}{}{}: form-data; name={}",
                    constants::BOUNDARY_EXT,
                    self.boundary,
                    constants::CRLF,
                    CONTENT_DISPOSITION,
                    part.key
                )
                .as_bytes(),
            );
            if let Some(filename) = part.filename {
                buf.extend_from_slice(format!("; filename=\"{}\"", filename).as_bytes());
            }
            if let Some(mime) = part.mime {
                buf.extend_from_slice(
                    format!("{}{}: {}", constants::CRLF, CONTENT_TYPE, mime).as_bytes(),
                );
            }
            for (k, v) in part.headers.iter() {
                buf.extend_from_slice(format!("{}{}: ", constants::CRLF, k).as_bytes());
                buf.extend_from_slice(v.as_bytes());
            }

            buf.extend_from_slice(constants::CRLF_CRLF.as_bytes());
            buf.extend_from_slice(&part.value);
            buf.extend_from_slice(constants::CRLF.as_bytes());
        }
        buf.extend_from_slice(
            format!(
                "{}{}{}",
                constants::BOUNDARY_EXT,
                self.boundary,
                constants::BOUNDARY_EXT,
            )
            .as_bytes(),
        );
        buf
    }
}

fn generate_random_string(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub struct Part {
    pub key: String,
    pub value: Vec<u8>,
    pub filename: Option<String>,
    pub mime: Option<Mime>,
    pub headers: HeaderMap,
}

impl Part {
    pub fn new<S, V>(key: S, value: V) -> Self
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
    {
        Self {
            key: key.into(),
            value: value.into(),
            filename: None,
            mime: None,
            headers: HeaderMap::new(),
        }
    }

    pub fn file<S, P>(key: S, path: P) -> Result<Self>
    where
        S: Into<String>,
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let mut file = File::open(path)?;
        let mut buffer = vec![];
        file.read_to_end(&mut buffer)?;
        let part = Part::new(key, buffer).mime(mime);

        match path
            .file_name()
            .map(|filename| filename.to_string_lossy().to_string())
        {
            Some(name) => Ok(part.filename(name)),
            None => Ok(part),
        }
    }

    pub fn mime(mut self, mime: Mime) -> Self {
        self.mime = Some(mime);
        self
    }

    pub fn mime_str(mut self, mime: &str) -> Result<Self> {
        self.mime = Some(mime.parse()?);
        Ok(self)
    }

    pub fn filename<S: Into<String>>(mut self, name: S) -> Self {
        self.filename = Some(name.into());
        self
    }

    pub fn headers<K, V, I>(mut self, headers: I) -> Result<Self>
    where
        K: IntoHeaderName,
        V: TryInto<HeaderValue>,
        <V as TryInto<HeaderValue>>::Error: Into<Error>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (key, value) in headers.into_iter() {
            self.headers
                .insert(key, value.try_into().map_err(|e| e.into())?);
        }
        Ok(self)
    }
}

// ============================================================================
// Streaming Multipart Support
// ============================================================================

/// The content source for a streaming part - either in-memory bytes or a reader
pub enum StreamingContent {
    /// In-memory bytes (for small text fields)
    Bytes(Vec<u8>),
    /// A reader that streams content (for files)
    Reader(Box<dyn Read + Send>),
}

/// A multipart form part that can stream its content from a reader.
///
/// Unlike `Part` which requires all data in memory, `StreamingPart` can
/// stream content from any `impl Read` source (files, network streams, etc.)
pub struct StreamingPart {
    pub key: String,
    pub content: StreamingContent,
    pub filename: Option<String>,
    pub mime: Option<Mime>,
    pub headers: HeaderMap,
}

impl StreamingPart {
    /// Create a new streaming part with in-memory data.
    /// Use this for small text fields.
    pub fn text<S, V>(key: S, value: V) -> Self
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
    {
        Self {
            key: key.into(),
            content: StreamingContent::Bytes(value.into()),
            filename: None,
            mime: None,
            headers: HeaderMap::new(),
        }
    }

    /// Create a new streaming part from a reader.
    ///
    /// The reader will be streamed in chunks when building the request body,
    /// avoiding loading the entire content into memory.
    ///
    /// # Example
    /// ```ignore
    /// use std::fs::File;
    /// use waki::multipart::StreamingPart;
    ///
    /// let file = File::open("large_file.bin")?;
    /// let part = StreamingPart::from_reader("file", file)
    ///     .filename("large_file.bin")
    ///     .mime_str("application/octet-stream")?;
    /// ```
    pub fn from_reader<S, R>(key: S, reader: R) -> Self
    where
        S: Into<String>,
        R: Read + Send + 'static,
    {
        Self {
            key: key.into(),
            content: StreamingContent::Reader(Box::new(reader)),
            filename: None,
            mime: None,
            headers: HeaderMap::new(),
        }
    }

    /// Create a streaming part from a file path.
    /// Opens the file but does NOT read it into memory.
    pub fn file<S, P>(key: S, path: P) -> Result<Self>
    where
        S: Into<String>,
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let file = File::open(path)?;

        let mut part = Self::from_reader(key, file).mime(mime);

        if let Some(name) = path.file_name() {
            part = part.filename(name.to_string_lossy().to_string());
        }

        Ok(part)
    }

    pub fn mime(mut self, mime: Mime) -> Self {
        self.mime = Some(mime);
        self
    }

    pub fn mime_str(mut self, mime: &str) -> Result<Self> {
        self.mime = Some(mime.parse()?);
        Ok(self)
    }

    pub fn filename<S: Into<String>>(mut self, name: S) -> Self {
        self.filename = Some(name.into());
        self
    }

    pub fn headers<K, V, I>(mut self, headers: I) -> Result<Self>
    where
        K: IntoHeaderName,
        V: TryInto<HeaderValue>,
        <V as TryInto<HeaderValue>>::Error: Into<Error>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (key, value) in headers.into_iter() {
            self.headers
                .insert(key, value.try_into().map_err(|e| e.into())?);
        }
        Ok(self)
    }

    /// Build the header portion of this part (everything before the content)
    fn build_header(&self, boundary: &str) -> Vec<u8> {
        let mut buf = Vec::new();

        buf.extend_from_slice(
            format!(
                "{}{}{}{}: form-data; name={}",
                constants::BOUNDARY_EXT,
                boundary,
                constants::CRLF,
                CONTENT_DISPOSITION,
                self.key
            )
            .as_bytes(),
        );

        if let Some(ref filename) = self.filename {
            buf.extend_from_slice(format!("; filename=\"{}\"", filename).as_bytes());
        }

        if let Some(ref mime) = self.mime {
            buf.extend_from_slice(
                format!("{}{}: {}", constants::CRLF, CONTENT_TYPE, mime).as_bytes(),
            );
        }

        for (k, v) in self.headers.iter() {
            buf.extend_from_slice(format!("{}{}: ", constants::CRLF, k).as_bytes());
            buf.extend_from_slice(v.as_bytes());
        }

        buf.extend_from_slice(constants::CRLF_CRLF.as_bytes());
        buf
    }
}

/// A multipart form that streams its content instead of building everything in memory.
///
/// Use this when uploading large files to avoid memory pressure.
pub struct StreamingForm {
    parts: Vec<StreamingPart>,
    boundary: String,
}

impl Default for StreamingForm {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingForm {
    pub fn new() -> Self {
        Self {
            parts: vec![],
            boundary: format!("--FormBoundary{}", generate_random_string(10)),
        }
    }

    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// Add a text field (small, in-memory data)
    pub fn text<S, V>(mut self, key: S, value: V) -> Self
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
    {
        self.parts.push(StreamingPart::text(key, value));
        self
    }

    /// Add a streaming part
    pub fn part(mut self, part: StreamingPart) -> Self {
        self.parts.push(part);
        self
    }

    /// Add a file part that will be streamed from disk
    pub fn file<S, P>(mut self, key: S, path: P) -> Result<Self>
    where
        S: Into<String>,
        P: AsRef<Path>,
    {
        self.parts.push(StreamingPart::file(key, path)?);
        Ok(self)
    }

    /// Convert this form into a reader that streams the multipart body.
    ///
    /// This allows the request body to be written in chunks without
    /// loading the entire form into memory.
    pub fn into_reader(self) -> StreamingFormReader {
        StreamingFormReader::new(self.parts, self.boundary)
    }
}

/// A reader that streams multipart form data.
///
/// Implements `std::io::Read` so it can be used with streaming body APIs.
pub struct StreamingFormReader {
    parts: std::collections::VecDeque<StreamingPart>,
    boundary: String,
    state: StreamingFormReaderState,
    pending_bytes: Vec<u8>,
    pending_offset: usize,
}

enum StreamingFormReaderState {
    /// Writing header bytes for the current part
    PartHeader,
    /// Streaming content from current part
    PartContent,
    /// Writing CRLF after part content
    PartTrailer,
    /// Writing final boundary
    Finished,
    /// Done, no more data
    Done,
}

impl StreamingFormReader {
    fn new(parts: Vec<StreamingPart>, boundary: String) -> Self {
        Self {
            parts: parts.into(),
            boundary,
            state: StreamingFormReaderState::PartHeader,
            pending_bytes: Vec::new(),
            pending_offset: 0,
        }
    }

    fn drain_pending(&mut self, buf: &mut [u8]) -> usize {
        if self.pending_offset >= self.pending_bytes.len() {
            return 0;
        }

        let remaining = &self.pending_bytes[self.pending_offset..];
        let to_copy = std::cmp::min(remaining.len(), buf.len());
        buf[..to_copy].copy_from_slice(&remaining[..to_copy]);
        self.pending_offset += to_copy;

        // Clear pending if fully drained
        if self.pending_offset >= self.pending_bytes.len() {
            self.pending_bytes.clear();
            self.pending_offset = 0;
        }

        to_copy
    }

    fn set_pending(&mut self, bytes: Vec<u8>) {
        self.pending_bytes = bytes;
        self.pending_offset = 0;
    }
}

impl Read for StreamingFormReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // First drain any pending bytes
        let drained = self.drain_pending(buf);
        if drained > 0 {
            return Ok(drained);
        }

        loop {
            match self.state {
                StreamingFormReaderState::PartHeader => {
                    if let Some(part) = self.parts.front() {
                        let header = part.build_header(&self.boundary);
                        self.set_pending(header);
                        self.state = StreamingFormReaderState::PartContent;
                        return Ok(self.drain_pending(buf));
                    } else {
                        // No more parts, write final boundary
                        self.state = StreamingFormReaderState::Finished;
                    }
                }
                StreamingFormReaderState::PartContent => {
                    if let Some(part) = self.parts.front_mut() {
                        match &mut part.content {
                            StreamingContent::Bytes(bytes) => {
                                if bytes.is_empty() {
                                    self.state = StreamingFormReaderState::PartTrailer;
                                } else {
                                    let to_copy = std::cmp::min(bytes.len(), buf.len());
                                    buf[..to_copy].copy_from_slice(&bytes[..to_copy]);
                                    bytes.drain(..to_copy);
                                    if bytes.is_empty() {
                                        self.state = StreamingFormReaderState::PartTrailer;
                                    }
                                    return Ok(to_copy);
                                }
                            }
                            StreamingContent::Reader(reader) => {
                                let n = reader.read(buf)?;
                                if n == 0 {
                                    // EOF, move to trailer
                                    self.state = StreamingFormReaderState::PartTrailer;
                                } else {
                                    return Ok(n);
                                }
                            }
                        }
                    } else {
                        self.state = StreamingFormReaderState::Finished;
                    }
                }
                StreamingFormReaderState::PartTrailer => {
                    // Pop the current part and write CRLF
                    self.parts.pop_front();
                    self.set_pending(constants::CRLF.as_bytes().to_vec());
                    self.state = StreamingFormReaderState::PartHeader;
                    return Ok(self.drain_pending(buf));
                }
                StreamingFormReaderState::Finished => {
                    let final_boundary = format!(
                        "{}{}{}",
                        constants::BOUNDARY_EXT,
                        self.boundary,
                        constants::BOUNDARY_EXT,
                    );
                    self.set_pending(final_boundary.into_bytes());
                    self.state = StreamingFormReaderState::Done;
                    return Ok(self.drain_pending(buf));
                }
                StreamingFormReaderState::Done => {
                    return Ok(0); // EOF
                }
            }
        }
    }
}
