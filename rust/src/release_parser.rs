#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedReleaseTitle {
    pub anime_name: String,
    pub season: i32,
    pub episode: i32,
    pub fansub: Option<String>,
    pub quality: Option<String>,
    pub languages: Vec<String>,
}

pub fn parse_release_title(title: &str) -> Option<ParsedReleaseTitle> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let (anime_end, season, episode) = find_season_episode(title)
        .or_else(|| find_trailing_episode(title).map(|(end, episode)| (end, 1, episode)))?;
    let (anime_name, fansub) = clean_anime_name(&title[..anime_end]);
    if anime_name.is_empty() {
        return None;
    }

    let lowercase = title.to_ascii_lowercase();
    let uppercase = title.to_ascii_uppercase();
    let quality = ["2160p", "1080p", "720p", "480p", "360p"]
        .into_iter()
        .find(|value| lowercase.contains(value))
        .map(str::to_string);
    let languages = [("CHS", "简"), ("CHT", "繁"), ("JPN", "日"), ("ENG", "英")]
        .into_iter()
        .filter(|(marker, _)| uppercase.contains(marker))
        .map(|(_, language)| language.to_string())
        .collect();

    Some(ParsedReleaseTitle {
        anime_name,
        season,
        episode,
        fansub,
        quality,
        languages,
    })
}

fn find_season_episode(title: &str) -> Option<(usize, i32, i32)> {
    let bytes = title.as_bytes();
    for index in 0..bytes.len() {
        if !matches!(bytes[index], b's' | b'S')
            || index > 0 && bytes[index - 1].is_ascii_alphanumeric()
        {
            continue;
        }
        let Some((season, next)) = parse_digits(bytes, index + 1, 2) else {
            continue;
        };
        if !matches!(bytes.get(next), Some(b'e' | b'E')) {
            continue;
        }
        let Some((episode, end)) = parse_digits(bytes, next + 1, 4) else {
            continue;
        };
        if season > 99 || episode < 1 || bytes.get(end).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let name_end = title[..index]
            .trim_end_matches([' ', '-', '_', '.', '　'])
            .len();
        return Some((name_end, season, episode));
    }

    let mut search_from = 0;
    while let Some(relative) = title[search_from..].find('第') {
        let season_start = search_from + relative + '第'.len_utf8();
        let Some((season, season_end)) = parse_unicode_number(title, season_start) else {
            search_from = season_start;
            continue;
        };
        if !title[season_end..].starts_with('季') {
            search_from = season_end;
            continue;
        }
        let episode_marker = season_end + '季'.len_utf8();
        let episode_start = if title[episode_marker..].starts_with('第') {
            episode_marker + '第'.len_utf8()
        } else {
            episode_marker
        };
        let Some((episode, episode_end)) = parse_unicode_number(title, episode_start) else {
            search_from = episode_start;
            continue;
        };
        if !matches!(
            title[episode_end..].chars().next(),
            Some('集' | '话' | '話')
        ) {
            search_from = episode_end;
            continue;
        }
        let name_end = title[..search_from + relative]
            .trim_end_matches([' ', '-', '_', '.', '　'])
            .len();
        return Some((name_end, season, episode));
    }

    let mut search_from = 0;
    while let Some(relative) = title[search_from..].find('第') {
        let marker = search_from + relative;
        let episode_start = marker + '第'.len_utf8();
        let Some((episode, episode_end)) = parse_unicode_number(title, episode_start) else {
            search_from = episode_start;
            continue;
        };
        if !matches!(
            title[episode_end..].chars().next(),
            Some('集' | '话' | '話')
        ) {
            search_from = episode_end;
            continue;
        }
        let name_end = title[..marker]
            .trim_end_matches([' ', '-', '_', '.', '　'])
            .len();
        return Some((name_end, 1, episode));
    }
    None
}

fn find_trailing_episode(title: &str) -> Option<(usize, i32)> {
    let mut end = title.trim_end().len();
    loop {
        let suffix = &title[..end];
        if !suffix.ends_with(']') {
            break;
        }
        let start = suffix.rfind('[')?;
        end = title[..start].trim_end().len();
    }

    let bytes = title.as_bytes();
    let mut number_start = end;
    while number_start > 0 && bytes[number_start - 1].is_ascii_digit() {
        number_start -= 1;
    }
    let digits = end - number_start;
    if !(1..=4).contains(&digits) || number_start == 0 {
        return None;
    }
    let separator = bytes[number_start - 1];
    if !matches!(separator, b' ' | b'-' | b'_' | b'.') {
        return None;
    }
    let episode = title[number_start..end].parse().ok()?;
    if episode < 1 {
        return None;
    }
    let anime_end = title[..number_start - 1]
        .trim_end_matches([' ', '-', '_', '.', '　'])
        .len();
    Some((anime_end, episode))
}

fn parse_digits(bytes: &[u8], start: usize, maximum: usize) -> Option<(i32, usize)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() && end - start < maximum {
        end += 1;
    }
    if end == start || bytes.get(end).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let value = std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()?;
    Some((value, end))
}

fn parse_unicode_number(value: &str, start: usize) -> Option<(i32, usize)> {
    let mut end = start;
    for (offset, character) in value[start..].char_indices() {
        if !character.is_ascii_digit() {
            break;
        }
        end = start + offset + character.len_utf8();
        if end - start > 4 {
            return None;
        }
    }
    (end > start)
        .then(|| value[start..end].parse().ok().map(|number| (number, end)))
        .flatten()
}

fn clean_anime_name(value: &str) -> (String, Option<String>) {
    let mut name = value.trim();
    let mut fansub = None;
    if let Some(rest) = name.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            fansub = Some(rest[..end].trim().to_string()).filter(|value| !value.is_empty());
            name = rest[end + 1..].trim();
        }
    } else if let Some(rest) = name.strip_prefix('【') {
        if let Some(end) = rest.find('】') {
            fansub = Some(rest[..end].trim().to_string()).filter(|value| !value.is_empty());
            name = rest[end + '】'.len_utf8()..].trim();
        }
    }
    (
        name.trim_matches(['-', '_', '.', ' ', '　']).to_string(),
        fansub,
    )
}

#[cfg(test)]
mod tests {
    use super::parse_release_title;

    #[test]
    fn parses_common_release_title_forms_without_guessing_episode_numbers() {
        let sxe = parse_release_title("[Group] Example Show S02E03 [1080p][CHT]").unwrap();
        assert_eq!(sxe.anime_name, "Example Show");
        assert_eq!((sxe.season, sxe.episode), (2, 3));
        assert_eq!(sxe.fansub.as_deref(), Some("Group"));
        assert_eq!(sxe.quality.as_deref(), Some("1080p"));
        assert_eq!(sxe.languages, ["繁"]);

        let chinese = parse_release_title("作品名 第2季第13集").unwrap();
        assert_eq!((chinese.season, chinese.episode), (2, 13));

        let trailing = parse_release_title("[ANi] 作品名 - 03 [1080P]").unwrap();
        assert_eq!(trailing.anime_name, "作品名");
        assert_eq!((trailing.season, trailing.episode), (1, 3));
        assert_eq!(trailing.fansub.as_deref(), Some("ANi"));

        assert!(parse_release_title("[Group] 作品名 [1080P]").is_none());
    }
}
