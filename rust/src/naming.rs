use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_RENAME_FORMAT: &str = "{anime_name} - S{season:02d}E{episode:02d}";

#[derive(Debug, Deserialize)]
pub struct NamingInput {
    pub anime_name: String,
    #[serde(default)]
    pub anime_name_override: Option<String>,
    #[serde(default)]
    pub download_directory_name_override: Option<String>,
    #[serde(default)]
    pub season: Option<i32>,
    #[serde(default)]
    pub episode: Option<i32>,
    #[serde(default)]
    pub filename_season_override: Option<i32>,
    #[serde(default)]
    pub directory_season_override: Option<i32>,
    #[serde(default)]
    pub extension: Option<String>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub rename_format: Option<String>,
    #[serde(default)]
    pub fansub: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct DownloadPlan {
    pub display_name: String,
    pub identity_season: i32,
    pub filename_season: i32,
    pub directory_season: i32,
    pub episode: i32,
    pub filename: String,
    pub directory: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("anime name is missing")]
    MissingAnimeName,
    #[error("episode number is missing or invalid")]
    MissingEpisode,
    #[error("season number is invalid")]
    InvalidSeason,
    #[error("filename extension is invalid")]
    InvalidExtension,
    #[error("directory name must be a single safe component")]
    UnsafeDirectoryComponent,
    #[error("rename format is invalid")]
    InvalidRenameFormat,
    #[error("rename format produces a path separator or NUL")]
    UnsafeRenameFormat,
    #[error("global download path must be an absolute safe path")]
    UnsafeBasePath,
}

impl PlanError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingAnimeName => "missing_anime_name",
            Self::MissingEpisode => "missing_episode",
            Self::InvalidSeason => "invalid_season",
            Self::InvalidExtension => "invalid_extension",
            Self::UnsafeDirectoryComponent => "unsafe_directory_component",
            Self::InvalidRenameFormat => "invalid_rename_format",
            Self::UnsafeRenameFormat => "unsafe_rename_format",
            Self::UnsafeBasePath => "unsafe_base_path",
        }
    }
}

pub fn build_plan(base_path: &str, input: NamingInput) -> Result<DownloadPlan, PlanError> {
    let identity_season = input.season.unwrap_or(1);
    let episode = input
        .episode
        .filter(|episode| *episode > 0)
        .ok_or(PlanError::MissingEpisode)?;
    validate_season(identity_season)?;

    let filename_season = resolve_override(input.filename_season_override, identity_season)?;
    let directory_season = resolve_override(input.directory_season_override, identity_season)?;
    let base_path = safe_base_path(base_path)?;

    let display_name = safe_component(
        input
            .anime_name_override
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&input.anime_name),
    )?;
    let directory_name_override = input
        .download_directory_name_override
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let directory_name = match directory_name_override {
        Some(value) => safe_directory_component(value)?,
        None => safe_component(&input.anime_name)?,
    };
    let extension = safe_extension(input.extension.as_deref().unwrap_or(".mp4"))?;
    let version = input.version.unwrap_or(1).max(1);
    let rename_format = input
        .rename_format
        .as_deref()
        .unwrap_or(DEFAULT_RENAME_FORMAT);
    let mut stem = render_stem(
        rename_format,
        &FormatValues {
            anime_name: &display_name,
            season: filename_season,
            episode,
            fansub: input.fansub.as_deref().unwrap_or(""),
            quality: input.quality.as_deref().unwrap_or(""),
            languages: &input.languages.join(""),
            version,
        },
    )?;
    if version > 1 {
        stem.push_str(&format!(" v{version}"));
    }
    let filename = format!("{}{extension}", safe_component(&stem)?);
    let directory = join_remote_path(
        &base_path,
        &directory_name,
        &format!("Season {directory_season}"),
    );

    Ok(DownloadPlan {
        display_name,
        identity_season,
        filename_season,
        directory_season,
        episode,
        filename,
        directory,
    })
}

pub fn validate_rename_format(rename_format: &str) -> Result<(), PlanError> {
    let sample = FormatValues {
        anime_name: "示例番剧",
        season: 1,
        episode: 1,
        fansub: "字幕组",
        quality: "1080p",
        languages: "简",
        version: 1,
    };
    render_stem(rename_format, &sample).map(|_| ())
}

fn validate_season(season: i32) -> Result<(), PlanError> {
    if (0..=99).contains(&season) {
        Ok(())
    } else {
        Err(PlanError::InvalidSeason)
    }
}

fn resolve_override(value: Option<i32>, fallback: i32) -> Result<i32, PlanError> {
    match value.filter(|value| *value != 0) {
        Some(value) if (1..=99).contains(&value) => Ok(value),
        Some(_) => Err(PlanError::InvalidSeason),
        None => Ok(fallback),
    }
}

fn safe_component(value: &str) -> Result<String, PlanError> {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches([' ', '.']).to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(PlanError::MissingAnimeName);
    }
    Ok(cleaned)
}

fn safe_directory_component(value: &str) -> Result<String, PlanError> {
    if value.trim() == "."
        || value.trim() == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(PlanError::UnsafeDirectoryComponent);
    }
    safe_component(value).map_err(|_| PlanError::UnsafeDirectoryComponent)
}

fn safe_extension(value: &str) -> Result<&str, PlanError> {
    let Some(extension) = value.strip_prefix('.') else {
        return Err(PlanError::InvalidExtension);
    };
    if extension.is_empty()
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(PlanError::InvalidExtension);
    }
    Ok(value)
}

fn safe_base_path(value: &str) -> Result<String, PlanError> {
    if !value.starts_with('/') || value.contains('\\') || value.contains('\0') {
        return Err(PlanError::UnsafeBasePath);
    }
    let components = value.split('/').filter(|component| !component.is_empty());
    let mut safe = Vec::new();
    for component in components {
        if component == "." || component == ".." {
            return Err(PlanError::UnsafeBasePath);
        }
        safe.push(component);
    }
    Ok(format!("/{}", safe.join("/")))
}

fn join_remote_path(base: &str, directory: &str, season: &str) -> String {
    if base == "/" {
        format!("/{directory}/{season}")
    } else {
        format!("{base}/{directory}/{season}")
    }
}

struct FormatValues<'a> {
    anime_name: &'a str,
    season: i32,
    episode: i32,
    fansub: &'a str,
    quality: &'a str,
    languages: &'a str,
    version: i32,
}

fn render_stem(format: &str, values: &FormatValues<'_>) -> Result<String, PlanError> {
    if format.trim().is_empty() {
        return Err(PlanError::InvalidRenameFormat);
    }
    let mut output = String::new();
    let mut characters = format.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '{' if characters.peek() == Some(&'{') => {
                characters.next();
                output.push('{');
            }
            '}' if characters.peek() == Some(&'}') => {
                characters.next();
                output.push('}');
            }
            '{' => {
                let mut field = String::new();
                loop {
                    match characters.next() {
                        Some('}') => break,
                        Some('{') | None => return Err(PlanError::InvalidRenameFormat),
                        Some(character) => field.push(character),
                    }
                }
                output.push_str(&render_field(&field, values)?);
            }
            '}' => return Err(PlanError::InvalidRenameFormat),
            character => output.push(character),
        }
    }
    if output.trim().is_empty() {
        return Err(PlanError::InvalidRenameFormat);
    }
    if output.contains('/') || output.contains('\\') || output.contains('\0') {
        return Err(PlanError::UnsafeRenameFormat);
    }
    Ok(output.trim().to_string())
}

fn render_field(field: &str, values: &FormatValues<'_>) -> Result<String, PlanError> {
    let (name, specifier) = field
        .split_once(':')
        .map_or((field, ""), |(name, specifier)| (name, specifier));
    match name {
        "anime_name" => render_text(values.anime_name, specifier),
        "fansub" => render_text(values.fansub, specifier),
        "quality" => render_text(values.quality, specifier),
        "languages" => render_text(values.languages, specifier),
        "season" => render_number(values.season, specifier),
        "episode" => render_number(values.episode, specifier),
        "version" => render_number(values.version, specifier),
        _ => Err(PlanError::InvalidRenameFormat),
    }
}

fn render_text(value: &str, specifier: &str) -> Result<String, PlanError> {
    if specifier.is_empty() {
        Ok(value.to_string())
    } else {
        Err(PlanError::InvalidRenameFormat)
    }
}

fn render_number(value: i32, specifier: &str) -> Result<String, PlanError> {
    if specifier.is_empty() || specifier == "d" {
        return Ok(value.to_string());
    }
    let digits = specifier
        .strip_suffix('d')
        .ok_or(PlanError::InvalidRenameFormat)?;
    let zero_pad = digits.starts_with('0');
    let width = digits
        .trim_start_matches('0')
        .parse::<usize>()
        .map_err(|_| PlanError::InvalidRenameFormat)?;
    if width > 8 {
        return Err(PlanError::InvalidRenameFormat);
    }
    if zero_pad {
        Ok(format!("{value:0width$}"))
    } else {
        Ok(format!("{value:width$}"))
    }
}
