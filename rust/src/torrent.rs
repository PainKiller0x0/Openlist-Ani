use std::{collections::BTreeMap, sync::OnceLock, time::Duration};

use reqwest::Url;
use serde::Serialize;
use sha1::{Digest, Sha1};
use thiserror::Error;

const MAX_TORRENT_BYTES: usize = 10 * 1024 * 1024;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;
const MAX_BENCODE_DEPTH: usize = 64;
const MAX_BENCODE_NODES: usize = 100_000;

#[derive(Debug, Error)]
pub enum TorrentError {
    #[error("invalid torrent URL")]
    InvalidUrl,
    #[error("torrent request failed")]
    Request(Option<u16>),
    #[error("torrent file exceeds the 10 MiB limit")]
    TooLarge,
    #[error("invalid torrent metadata")]
    InvalidMetadata,
}

#[derive(Debug, Serialize)]
pub struct ResolveTorrentResponse {
    pub success: bool,
    pub message: String,
    pub title: Option<String>,
    pub source: Option<String>,
    pub file_count: Option<usize>,
    pub files: Vec<TorrentFile>,
}

impl ResolveTorrentResponse {
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            title: None,
            source: None,
            file_count: None,
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TorrentFile {
    pub name: String,
    pub size: u64,
}

pub async fn resolve_torrent(url: &str) -> Result<ResolveTorrentResponse, TorrentError> {
    let url = Url::parse(url.trim()).map_err(|_| TorrentError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(TorrentError::InvalidUrl);
    }

    let response = client()
        .get(url)
        .send()
        .await
        .map_err(|_| TorrentError::Request(None))?;
    if !response.status().is_success() {
        return Err(TorrentError::Request(response.status().as_u16().into()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TORRENT_BYTES as u64)
    {
        return Err(TorrentError::TooLarge);
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| TorrentError::Request(None))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_TORRENT_BYTES {
            return Err(TorrentError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    let metadata = parse_torrent(&body).ok_or(TorrentError::InvalidMetadata)?;
    Ok(ResolveTorrentResponse {
        success: true,
        message: "Resolved title from .torrent file metadata.".to_string(),
        title: Some(metadata.title),
        source: Some("torrent_file".to_string()),
        file_count: Some(metadata.files.len()),
        files: metadata.files,
    })
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .user_agent("Mozilla/5.0 (OpenList-Ani; torrent resolver)")
            .build()
            .expect("valid static HTTP client configuration")
    })
}

struct TorrentMetadata {
    title: String,
    files: Vec<TorrentFile>,
}

fn parse_torrent(input: &[u8]) -> Option<TorrentMetadata> {
    parse_torrent_with_hash(input).map(|(metadata, _)| metadata)
}

fn parse_torrent_with_hash(input: &[u8]) -> Option<(TorrentMetadata, String)> {
    if input.len() > MAX_TORRENT_BYTES {
        return None;
    }
    let mut parser = BencodeParser::new(input);
    if parser.peek()? != b'd' {
        return None;
    }
    parser.position += 1;
    let mut root = BTreeMap::new();
    let mut info_range = None;
    while parser.peek()? != b'e' {
        let key = parser.parse_bytes()?;
        let start = parser.position;
        let value = parser.parse_value(1)?;
        let end = parser.position;
        if key == b"info" {
            info_range = Some((start, end));
        }
        if root.insert(key, value).is_some() {
            return None;
        }
    }
    parser.position += 1;
    if parser.position != input.len() {
        return None;
    }

    let info = get_dict(root.get(b"info".as_slice())?)?;
    let title = get_bytes(info, b"name.utf-8")
        .or_else(|| get_bytes(info, b"name"))
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())?;
    let files = if let Some(entries) = info.get(b"files".as_slice()) {
        let entries = get_list(entries)?;
        let mut files = Vec::with_capacity(entries.len());
        for entry in entries {
            let entry = get_dict(entry)?;
            let size = get_integer(entry, b"length")?;
            if size < 0 {
                return None;
            }
            let path =
                get_list_value(entry, b"path.utf-8").or_else(|| get_list_value(entry, b"path"))?;
            if path.is_empty() {
                return None;
            }
            let mut parts = vec![title.clone()];
            for component in path {
                let component = String::from_utf8_lossy(get_bytes_value(component)?).into_owned();
                if component.is_empty()
                    || component == "."
                    || component == ".."
                    || component.contains('/')
                    || component.contains('\\')
                    || component.contains('\0')
                {
                    return None;
                }
                parts.push(component);
            }
            files.push(TorrentFile {
                name: parts.join("/"),
                size: size as u64,
            });
        }
        if files.is_empty() {
            return None;
        }
        files
    } else {
        let size = get_integer(info, b"length")?;
        if size < 0 {
            return None;
        }
        vec![TorrentFile {
            name: title.clone(),
            size: size as u64,
        }]
    };
    let (start, end) = info_range?;
    let info_hash = format!("{:x}", Sha1::digest(&input[start..end]));
    Some((TorrentMetadata { title, files }, info_hash))
}

enum BencodeValue {
    Bytes(Vec<u8>),
    Integer(i64),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

struct BencodeParser<'a> {
    input: &'a [u8],
    position: usize,
    nodes: usize,
}

impl<'a> BencodeParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            position: 0,
            nodes: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn parse_value(&mut self, depth: usize) -> Option<BencodeValue> {
        if depth > MAX_BENCODE_DEPTH || self.nodes >= MAX_BENCODE_NODES {
            return None;
        }
        self.nodes += 1;
        match self.peek()? {
            b'i' => self.parse_integer().map(BencodeValue::Integer),
            b'l' => {
                self.position += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    items.push(self.parse_value(depth + 1)?);
                }
                self.position += 1;
                Some(BencodeValue::List(items))
            }
            b'd' => {
                self.position += 1;
                let mut entries = BTreeMap::new();
                while self.peek()? != b'e' {
                    let key = self.parse_bytes()?;
                    let value = self.parse_value(depth + 1)?;
                    if entries.insert(key, value).is_some() {
                        return None;
                    }
                }
                self.position += 1;
                Some(BencodeValue::Dict(entries))
            }
            b'0'..=b'9' => self.parse_bytes().map(BencodeValue::Bytes),
            _ => None,
        }
    }

    fn parse_integer(&mut self) -> Option<i64> {
        self.position += 1;
        let start = self.position;
        while self.peek()? != b'e' {
            self.position += 1;
        }
        let raw = self.input.get(start..self.position)?;
        if raw.is_empty()
            || raw == b"-0"
            || (raw.len() > 1 && raw[0] == b'0')
            || (raw.len() > 2 && raw[0] == b'-' && raw[1] == b'0')
        {
            return None;
        }
        let value = std::str::from_utf8(raw).ok()?.parse().ok()?;
        self.position += 1;
        Some(value)
    }

    fn parse_bytes(&mut self) -> Option<Vec<u8>> {
        let start = self.position;
        while self.peek()?.is_ascii_digit() {
            self.position += 1;
        }
        if self.peek()? != b':' {
            return None;
        }
        let digits = self.input.get(start..self.position)?;
        if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') {
            return None;
        }
        let length: usize = std::str::from_utf8(digits).ok()?.parse().ok()?;
        self.position += 1;
        let end = self.position.checked_add(length)?;
        let value = self.input.get(self.position..end)?.to_vec();
        self.position = end;
        Some(value)
    }
}

fn get_dict(value: &BencodeValue) -> Option<&BTreeMap<Vec<u8>, BencodeValue>> {
    match value {
        BencodeValue::Dict(dict) => Some(dict),
        _ => None,
    }
}

fn get_list(value: &BencodeValue) -> Option<&[BencodeValue]> {
    match value {
        BencodeValue::List(values) => Some(values),
        _ => None,
    }
}

fn get_bytes_value(value: &BencodeValue) -> Option<&[u8]> {
    match value {
        BencodeValue::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

fn get_bytes<'a>(dict: &'a BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Option<&'a [u8]> {
    get_bytes_value(dict.get(key)?)
}

fn get_integer(dict: &BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Option<i64> {
    match dict.get(key)? {
        BencodeValue::Integer(value) => Some(*value),
        _ => None,
    }
}

fn get_list_value<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
) -> Option<&'a [BencodeValue]> {
    get_list(dict.get(key)?)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TORRENT_BYTES, parse_torrent, parse_torrent_with_hash};

    #[test]
    fn parses_single_file_fixture_and_matches_info_hash() {
        let blob = include_bytes!("../fixtures/torrent-single-v1.torrent");
        let (metadata, info_hash) = parse_torrent_with_hash(blob).unwrap();

        assert_eq!(metadata.title, "synthetic.mp4");
        assert_eq!(info_hash, "b54bdc092d6746ee0207c08e7550b5a004574246");
        assert_eq!(metadata.files.len(), 1);
        assert_eq!(metadata.files[0].name, "synthetic.mp4");
        assert_eq!(metadata.files[0].size, 0);
    }

    #[test]
    fn parses_nonzero_metadata_seed_without_loading_payload() {
        let blob = include_bytes!("../fixtures/torrent-metadata-seed-v1.torrent");
        let (metadata, info_hash) = parse_torrent_with_hash(blob).unwrap();

        assert_eq!(metadata.title, "synthetic-nonzero.mp4");
        assert_eq!(info_hash, "ec35f3f9dd458652ef8f08bac161b5e732be1e47");
        assert_eq!(metadata.files[0].size, 131_072);
    }

    #[test]
    fn multi_file_paths_include_the_torrent_root() {
        let blob = b"d4:infod5:filesld6:lengthi11e4:pathl11:episode.mkveee4:name4:rootee";
        let metadata = parse_torrent(blob).unwrap();

        assert_eq!(metadata.title, "root");
        assert_eq!(metadata.files[0].name, "root/episode.mkv");
        assert_eq!(metadata.files[0].size, 11);
    }

    #[test]
    fn rejects_malformed_or_incomplete_metadata() {
        assert!(parse_torrent(b"d4:infoe").is_none());
        assert!(parse_torrent(b"d4:infod4:name3:fooeejunk").is_none());
        assert!(parse_torrent(b"d4:infod4:name3:fooe").is_none());
        assert!(parse_torrent(&vec![b'a'; MAX_TORRENT_BYTES + 1]).is_none());
    }
}
