use std::collections::HashSet;
use std::time::Duration;

use reqwest::Url;
use serde::Serialize;
use thiserror::Error;

const MAX_HTML_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum MikanError {
    #[error("Mikan base URL must be a complete HTTP(S) URL")]
    InvalidBaseUrl,
    #[error("Mikan request failed")]
    Request(#[from] reqwest::Error),
    #[error("Mikan response exceeded the 5 MiB limit")]
    TooLarge,
    #[error("Mikan response was not valid UTF-8")]
    InvalidEncoding,
}

#[derive(Debug, Serialize)]
pub struct MikanSearchResult {
    pub bangumi_id: i64,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct MikanGroup {
    pub id: i64,
    pub name: String,
    pub release_count: usize,
    pub rss_url: String,
}

pub async fn search(base_url: &str, keyword: &str) -> Result<Vec<MikanSearchResult>, MikanError> {
    let base = parse_base_url(base_url)?;
    let mut url = base
        .join("/Home/Search")
        .map_err(|_| MikanError::InvalidBaseUrl)?;
    url.query_pairs_mut().append_pair("searchstr", keyword);
    let html = fetch_html(url).await?;
    Ok(parse_search_results(&html, &base))
}

pub async fn groups(base_url: &str, bangumi_id: i64) -> Result<Vec<MikanGroup>, MikanError> {
    let base = parse_base_url(base_url)?;
    let page_url = base
        .join(&format!("/Home/Bangumi/{bangumi_id}"))
        .map_err(|_| MikanError::InvalidBaseUrl)?;
    let html = fetch_html(page_url).await?;
    Ok(parse_groups(&html, &base, bangumi_id))
}

pub fn rss_url(
    base_url: &str,
    bangumi_id: i64,
    subgroup_id: Option<i64>,
) -> Result<String, MikanError> {
    let base = parse_base_url(base_url)?;
    let mut url = base
        .join("/RSS/Bangumi")
        .map_err(|_| MikanError::InvalidBaseUrl)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("bangumiId", &bangumi_id.to_string());
        if let Some(subgroup_id) = subgroup_id {
            query.append_pair("subgroupid", &subgroup_id.to_string());
        }
    }
    Ok(url.to_string())
}

fn parse_base_url(value: &str) -> Result<Url, MikanError> {
    let url = Url::parse(value.trim()).map_err(|_| MikanError::InvalidBaseUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(MikanError::InvalidBaseUrl);
    }
    Ok(url)
}

async fn fetch_html(url: Url) -> Result<String, MikanError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("openlist-ani/1.0 (https://github.com/Openlist-Ani)")
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTML_BYTES as u64)
    {
        return Err(MikanError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_HTML_BYTES {
            return Err(MikanError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| MikanError::InvalidEncoding)
}

fn parse_search_results(html: &str, base: &Url) -> Vec<MikanSearchResult> {
    let mut seen = HashSet::new();
    anchor_elements(html)
        .into_iter()
        .filter_map(|(opening, inner)| {
            let href = attribute(opening, "href")?;
            let id = href
                .split("/Home/Bangumi/")
                .nth(1)?
                .split(|character: char| !character.is_ascii_digit())
                .next()?
                .parse::<i64>()
                .ok()?;
            let name = text_content(inner);
            if id <= 0 || name.is_empty() || !seen.insert(id) {
                return None;
            }
            let url = base.join(&href).ok()?.to_string();
            Some(MikanSearchResult {
                bangumi_id: id,
                name,
                url,
            })
        })
        .collect()
}

fn parse_groups(html: &str, base: &Url, bangumi_id: i64) -> Vec<MikanGroup> {
    let mut seen = HashSet::new();
    anchor_elements(html)
        .into_iter()
        .filter_map(|(opening, inner)| {
            let class = attribute(opening, "class").unwrap_or_default();
            if !class
                .split_ascii_whitespace()
                .any(|value| value == "subgroup-name")
            {
                return None;
            }
            let anchor = attribute(opening, "data-anchor")?;
            let id = anchor.strip_prefix('#')?.parse::<i64>().ok()?;
            let name = text_content(inner);
            if id <= 0 || name.is_empty() || !seen.insert(id) {
                return None;
            }
            let release_count = release_count_for_group(html, id);
            let rss_url = rss_url(base.as_str(), bangumi_id, Some(id)).ok()?;
            Some(MikanGroup {
                id,
                name,
                release_count,
                rss_url,
            })
        })
        .collect()
}

fn release_count_for_group(html: &str, group_id: i64) -> usize {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start) = find_open_tag(&lower, "div", cursor) {
        let Some(open_end) = tag_end(&lower, start) else {
            return 0;
        };
        if attribute(&html[start..=open_end], "id").as_deref()
            != Some(group_id.to_string().as_str())
        {
            cursor = open_end + 1;
            continue;
        }
        let Some(header_end) = matching_tag_end(&lower, "div", open_end + 1) else {
            return 0;
        };
        let Some(table_start) = find_open_tag(&lower, "div", header_end + 1) else {
            return 0;
        };
        let Some(table_open_end) = tag_end(&lower, table_start) else {
            return 0;
        };
        let table_opening = &html[table_start..=table_open_end];
        if !attribute(table_opening, "class")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .any(|value| value == "episode-table")
        {
            return 0;
        }
        let Some(table_end) = matching_tag_end(&lower, "div", table_open_end + 1) else {
            return 0;
        };
        return count_open_tags(&lower[table_open_end + 1..table_end], "tr");
    }
    0
}

fn anchor_elements(html: &str) -> Vec<(&str, &str)> {
    let lower = html.to_ascii_lowercase();
    let mut elements = Vec::new();
    let mut cursor = 0;
    while let Some(start) = find_open_tag(&lower, "a", cursor) {
        let Some(open_end) = tag_end(&lower, start) else {
            break;
        };
        let Some(close_start) = lower[open_end + 1..].find("</a") else {
            break;
        };
        let close_start = open_end + 1 + close_start;
        let Some(close_end) = tag_end(&lower, close_start) else {
            break;
        };
        elements.push((&html[start..=open_end], &html[open_end + 1..close_start]));
        cursor = close_end + 1;
    }
    elements
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut cursor = 1;
    while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'>' {
        cursor += 1;
    }
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'>' | b'/')
        {
            cursor += 1;
        }
        if start == cursor {
            break;
        }
        let key = &tag[start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value = if cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b'"') {
            let quote = bytes[cursor];
            cursor += 1;
            let value_start = cursor;
            while cursor < bytes.len() && bytes[cursor] != quote {
                cursor += 1;
            }
            let value = &tag[value_start..cursor];
            cursor = cursor.saturating_add(1);
            value
        } else {
            let value_start = cursor;
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && bytes[cursor] != b'>'
            {
                cursor += 1;
            }
            &tag[value_start..cursor]
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(decode_entities(value));
        }
    }
    None
}

fn text_content(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(';') else {
            decoded.push_str(&rest[start..]);
            return decoded;
        };
        let entity_end = start + end;
        let entity = &rest[start + 1..entity_end];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                .or_else(|| {
                    entity
                        .strip_prefix('#')
                        .and_then(|digits| digits.parse().ok())
                })
                .and_then(char::from_u32),
        };
        if let Some(character) = replacement {
            decoded.push(character);
        } else {
            decoded.push_str(&rest[start..=entity_end]);
        }
        rest = &rest[entity_end + 1..];
    }
    decoded.push_str(rest);
    decoded
}

fn find_open_tag(lower: &str, tag: &str, from: usize) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut cursor = from;
    while let Some(relative) = lower[cursor..].find(&needle) {
        let start = cursor + relative;
        let next = lower.as_bytes().get(start + needle.len()).copied();
        if next.is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/')) {
            return Some(start);
        }
        cursor = start + needle.len();
    }
    None
}

fn tag_end(lower: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, byte) in lower.as_bytes()[start..].iter().copied().enumerate() {
        if let Some(end_quote) = quote {
            if byte == end_quote {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'>' {
            return Some(start + offset);
        }
    }
    None
}

fn matching_tag_end(lower: &str, tag: &str, mut cursor: usize) -> Option<usize> {
    let mut depth = 1usize;
    loop {
        let open = find_open_tag(lower, tag, cursor);
        let close_needle = format!("</{tag}");
        let close = lower[cursor..]
            .find(&close_needle)
            .map(|offset| cursor + offset);
        match (open, close) {
            (_, None) => return None,
            (Some(open), Some(close)) if open < close => {
                let end = tag_end(lower, open)?;
                if lower.as_bytes()[end.saturating_sub(1)] != b'/' {
                    depth += 1;
                }
                cursor = end + 1;
            }
            (_, Some(close)) => {
                let end = tag_end(lower, close)?;
                depth -= 1;
                if depth == 0 {
                    return Some(end);
                }
                cursor = end + 1;
            }
        }
    }
}

fn count_open_tags(lower: &str, tag: &str) -> usize {
    let mut cursor = 0;
    let mut count = 0;
    while let Some(start) = find_open_tag(lower, tag, cursor) {
        let Some(end) = tag_end(lower, start) else {
            break;
        };
        count += 1;
        cursor = end + 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::{parse_groups, parse_search_results, rss_url};
    use reqwest::Url;

    #[test]
    fn search_parser_handles_relative_links_entities_and_duplicates() {
        let html = r##"
            <a href="/Home/Bangumi/3992">尼古喵喵 &amp; Friends</a>
            <a class="duplicate" href='/Home/Bangumi/3992?from=search'>重复项</a>
            <a href="https://mikan.example/Home/Bangumi/5000">番剧&#x4E8C;</a>
        "##;
        let base = Url::parse("https://mikan.example/").unwrap();
        let results = parse_search_results(html, &base);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].bangumi_id, 3992);
        assert_eq!(results[0].name, "尼古喵喵 & Friends");
        assert_eq!(results[0].url, "https://mikan.example/Home/Bangumi/3992");
        assert_eq!(results[1].name, "番剧二");
    }

    #[test]
    fn group_parser_counts_release_rows_and_builds_custom_site_rss_urls() {
        let html = r##"
            <a class="subgroup-name" data-anchor="#359">ANi</a>
            <div id="359"></div>
            <div class="episode-table"><table><tbody>
                <tr><td>one</td></tr><tr><td>two</td></tr>
            </tbody></table></div>
            <a data-anchor="#359" class="subgroup-name">duplicate</a>
            <a data-anchor="#583" class="subgroup-name">字幕&amp;组</a>
            <div id="583"></div><div class="episode-table"><tr><td>one</td></tr></div>
        "##;
        let base = Url::parse("https://mikan.example/").unwrap();
        let groups = parse_groups(html, &base, 4064);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].id, 359);
        assert_eq!(groups[0].release_count, 2);
        assert_eq!(
            groups[0].rss_url,
            "https://mikan.example/RSS/Bangumi?bangumiId=4064&subgroupid=359"
        );
        assert_eq!(groups[1].name, "字幕&组");
        assert_eq!(groups[1].release_count, 1);
    }

    #[test]
    fn rss_url_omits_group_parameter_for_all_subgroups() {
        assert_eq!(
            rss_url("https://mikan.example", 4064, None).unwrap(),
            "https://mikan.example/RSS/Bangumi?bangumiId=4064"
        );
    }
}
