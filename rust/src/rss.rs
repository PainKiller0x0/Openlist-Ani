use reqwest::Url;
use roxmltree::{Document, Node};
use serde::Serialize;
use thiserror::Error;

const MAX_FEED_BYTES: usize = 10 * 1024 * 1024;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Error)]
pub enum RssError {
    #[error("RSS URL must be a complete HTTP(S) URL")]
    InvalidUrl,
    #[error("RSS request failed")]
    Request(#[from] reqwest::Error),
    #[error("RSS response exceeded the 10 MiB limit")]
    TooLarge,
    #[error("RSS response was not valid UTF-8")]
    InvalidEncoding,
    #[error("RSS or Atom XML could not be parsed")]
    InvalidXml,
}

#[derive(Debug, Serialize)]
pub struct ParseRssResponse {
    pub success: bool,
    pub message: String,
    pub total: usize,
    pub entries: Vec<ParseRssEntry>,
}

impl ParseRssResponse {
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            total: 0,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ParseRssEntry {
    pub index: usize,
    pub title: String,
    pub download_url: String,
    pub anime_name: Option<String>,
    pub episode: Option<i64>,
    pub fansub: Option<String>,
    pub quality: Option<String>,
    pub languages: Vec<String>,
}

pub async fn parse_rss(url: &str, limit: Option<i64>) -> Result<ParseRssResponse, RssError> {
    let parsed_url = Url::parse(url.trim()).map_err(|_| RssError::InvalidUrl)?;
    if !matches!(parsed_url.scheme(), "http" | "https") || parsed_url.host_str().is_none() {
        return Err(RssError::InvalidUrl);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .user_agent("OpenList-Ani/1.0")
        .build()?;
    let mut response = client.get(parsed_url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FEED_BYTES as u64)
    {
        return Err(RssError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_FEED_BYTES {
            return Err(RssError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let xml = std::str::from_utf8(&body).map_err(|_| RssError::InvalidEncoding)?;
    parse_feed_xml(url, xml, limit)
}

fn parse_feed_xml(url: &str, xml: &str, limit: Option<i64>) -> Result<ParseRssResponse, RssError> {
    let document = Document::parse(xml).map_err(|_| RssError::InvalidXml)?;
    let root = document.root_element();
    let source = source_kind(url);
    let mut entries = root
        .descendants()
        .filter(|node| node.is_element() && matches!(node.tag_name().name(), "item" | "entry"))
        .filter_map(|node| parse_entry(node, source))
        .enumerate()
        .map(|(index, mut entry)| {
            entry.index = index;
            entry
        })
        .collect::<Vec<_>>();
    let total = entries.len();
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        entries.truncate(limit as usize);
    }
    let message = if entries.len() < total {
        format!("Parsed {} of {total} entries", entries.len())
    } else {
        format!("Parsed {total} entries")
    };
    Ok(ParseRssResponse {
        success: true,
        message,
        total,
        entries,
    })
}

#[derive(Clone, Copy)]
enum SourceKind {
    Common,
    Mikan,
    Ani,
}

fn source_kind(url: &str) -> SourceKind {
    let host = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if ["mikanani.me", "mikanani.kas.pub", "mikanime.tv"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    {
        SourceKind::Mikan
    } else if ["ani.rip", "api.ani.rip"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    {
        SourceKind::Ani
    } else {
        SourceKind::Common
    }
}

struct Enclosure {
    href: String,
    media_type: String,
}

fn parse_entry(node: Node<'_, '_>, source: SourceKind) -> Option<ParseRssEntry> {
    let title = child_text(node, "title")?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let mut links = Vec::new();
    let mut enclosures = Vec::new();
    for child in node.children().filter(|child| child.is_element()) {
        let tag = child.tag_name().name();
        if tag == "enclosure" || (tag == "link" && child.attribute("rel") == Some("enclosure")) {
            if let Some(href) = child
                .attribute("url")
                .or_else(|| child.attribute("href"))
                .filter(|href| !href.trim().is_empty())
            {
                enclosures.push(Enclosure {
                    href: href.to_string(),
                    media_type: child.attribute("type").unwrap_or("").to_ascii_lowercase(),
                });
            }
        } else if tag == "link" {
            let link = child
                .attribute("href")
                .map(str::to_string)
                .or_else(|| child.text().map(str::to_string));
            if let Some(link) = link.filter(|link| !link.trim().is_empty()) {
                links.push(link.trim().to_string());
            }
        }
    }

    let download_url = match source {
        SourceKind::Ani => links
            .first()
            .cloned()
            .or_else(|| enclosures.first().map(|item| item.href.clone())),
        SourceKind::Mikan => enclosures
            .iter()
            .find(|item| item.media_type == "application/x-bittorrent")
            .map(|item| item.href.clone())
            .or_else(|| {
                links
                    .iter()
                    .find(|link| is_magnet_or_torrent(link))
                    .cloned()
            }),
        SourceKind::Common => enclosures
            .iter()
            .find(|item| {
                is_magnet_or_torrent(&item.href) || is_torrent_media_type(&item.media_type)
            })
            .map(|item| item.href.clone())
            .or_else(|| {
                links
                    .iter()
                    .find(|link| is_magnet_or_torrent(link))
                    .cloned()
            }),
    }?;

    if matches!(source, SourceKind::Mikan) && links.is_empty() {
        return None;
    }
    Some(ParseRssEntry {
        index: 0,
        title,
        download_url,
        anime_name: None,
        episode: None,
        fansub: matches!(source, SourceKind::Ani).then(|| "ANi".to_string()),
        quality: None,
        languages: Vec::new(),
    })
}

fn child_text<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
}

fn is_magnet_or_torrent(value: &str) -> bool {
    value.starts_with("magnet:") || torrent_path(value)
}

fn torrent_path(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .map(|url| url.path().to_ascii_lowercase().ends_with(".torrent"))
        .unwrap_or_else(|| {
            value
                .split(['?', '#'])
                .next()
                .unwrap_or(value)
                .ends_with(".torrent")
        })
}

fn is_torrent_media_type(value: &str) -> bool {
    matches!(value, "application/x-bittorrent" | "application/bittorrent")
}

#[cfg(test)]
mod tests {
    use super::parse_feed_xml;

    #[test]
    fn parses_rss_enclosures_in_order_and_applies_limit_after_filtering() {
        let xml = r#"<?xml version="1.0"?><rss><channel>
            <item><title>第一集</title><link>https://example.test/page/1</link><enclosure url="magnet:?xt=urn:btih:one" type="application/x-bittorrent"/></item>
            <item><title>网页入口</title><link>https://example.test/page/skip</link></item>
            <item><title>第二集</title><enclosure url="https://example.test/2.torrent?token=abc" type="application/octet-stream"/></item>
        </channel></rss>"#;

        let response = parse_feed_xml("https://example.test/feed.xml", xml, Some(1)).unwrap();

        assert_eq!(response.total, 2);
        assert_eq!(response.message, "Parsed 1 of 2 entries");
        assert_eq!(response.entries[0].index, 0);
        assert_eq!(response.entries[0].title, "第一集");
        assert_eq!(response.entries[0].download_url, "magnet:?xt=urn:btih:one");
    }

    #[test]
    fn parses_atom_links_and_ani_direct_downloads() {
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry>
            <title>Atom episode</title><link rel="alternate" href="https://example.test/page"/>
            <link rel="enclosure" type="application/x-bittorrent" href="https://example.test/a.torrent"/>
        </entry></feed>"#;
        let atom_result = parse_feed_xml("https://example.test/feed.xml", atom, None).unwrap();
        assert_eq!(
            atom_result.entries[0].download_url,
            "https://example.test/a.torrent"
        );

        let ani = r#"<rss><channel><item><title>[ANi] Show - 01</title><link>https://ani.rip/episode.mp4</link></item></channel></rss>"#;
        let ani_result = parse_feed_xml("https://ani.rip/rss", ani, None).unwrap();
        assert_eq!(
            ani_result.entries[0].download_url,
            "https://ani.rip/episode.mp4"
        );
        assert_eq!(ani_result.entries[0].fansub.as_deref(), Some("ANi"));
    }
}
