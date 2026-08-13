"""
Configuration management module.
Supports explicit loading and Pydantic validation.
"""

import os
import tomllib
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field, field_validator, model_validator
from tomlkit import dumps as toml_dumps

from openlist_ani.domain.anime_release import DEFAULT_RENAME_FORMAT
from openlist_ani.integrations.openlist import normalize_offline_download_tool_name
from openlist_ani.logger import FATAL_LEVEL, logger

DEFAULT_TMDB_API_KEY = "8ed20a12d9f37dcf9484a505c8be696c"
PLACEHOLDER_RSS_URL = "http://127.0.0.1:26667/empty.xml"


class PriorityConfig(BaseModel):
    """Configuration for release download priority filtering.

    Each field is an ordered list where earlier entries have higher priority.
    When a higher-priority release has already been downloaded for the same
    (anime_name, season, episode), lower-priority releases are skipped.

    The ``version`` field is exempt: a newer version is always downloaded
    regardless of priority rules.
    """

    field_order: list[str] = Field(
        default_factory=lambda: ["fansub", "quality", "languages"]
    )  # Order in which fields are compared; earlier fields take precedence
    fansub: list[str] = Field(
        default_factory=list
    )  # Fansub group priority, e.g. ["Fansub_A", "Fansub_B"]
    languages: list[str] = Field(default_factory=list)  # Language priority labels
    quality: list[str] = Field(
        default_factory=lambda: ["2160p", "1080p", "720p", "480p", "360p"]
    )  # Quality priority (high to low); set to [] to disable


class MetadataFilterConfig(BaseModel):
    """Configuration for metadata-based blacklist filtering.

    Each field is a list of values to exclude.  An RSS entry whose
    metadata matches any value in the corresponding list is filtered out.
    """

    exclude_fansub: list[str] = Field(default_factory=list)  # Fansub groups to exclude
    exclude_quality: list[str] = Field(
        default_factory=list
    )  # Quality values to exclude, e.g. ["480p"]
    exclude_languages: list[str] = Field(
        default_factory=list
    )  # Language labels to exclude
    exclude_patterns: list[str] = Field(
        default_factory=list
    )  # Regex patterns to exclude RSS entries by title


class RSSSubscription(BaseModel):
    """Persisted metadata for one RSS subscription."""

    url: str
    name: str = ""
    enabled: bool = True
    tmdb_id: int | None = None
    poster_url: str = ""
    exclude_patterns: list[str] = Field(
        default_factory=list,
        description="Regex patterns excluded only for this RSS subscription",
    )


class RSSConfig(BaseModel):
    urls: list[str] = Field(default_factory=list)
    subscriptions: list[RSSSubscription] = Field(default_factory=list)
    interval_time: int = 300  # RSS fetch interval in seconds (default: 5 minutes)
    strict: bool = (
        False  # Strict mode: filter entries whose rename stem matches existing downloads
    )
    filter: MetadataFilterConfig = MetadataFilterConfig()
    priority: PriorityConfig = PriorityConfig()

    @model_validator(mode="after")
    def _synchronise_urls_and_subscriptions(self) -> "RSSConfig":
        """Migrate legacy ``urls`` and keep active URLs runtime-compatible."""
        by_url = {item.url: item for item in self.subscriptions if item.url.strip()}
        for url in self.urls:
            if url.strip() and url not in by_url:
                by_url[url] = RSSSubscription(url=url)

        self.subscriptions = list(by_url.values())
        self.urls = [item.url for item in self.subscriptions if item.enabled]
        return self


class OpenListConfig(BaseModel):
    url: str = "http://localhost:5244"
    token: str = ""
    download_path: str = "/"
    offline_download_tool: str = "qBittorrent"
    rename_format: str = DEFAULT_RENAME_FORMAT

    @field_validator("offline_download_tool", mode="before")
    @classmethod
    def _validate_offline_download_tool(cls, value: str) -> str:
        if isinstance(value, str):
            normalized = value.strip()
            if not normalized:
                raise ValueError("offline_download_tool cannot be empty.")
            return normalize_offline_download_tool_name(normalized)
        return value


class DownloaderConfig(BaseModel):
    provider: str = "openlist"


class FileRenamerConfig(BaseModel):
    provider: str = "openlist"


class LLMConfig(BaseModel):
    provider_type: str = "openai"  # "openai" | "anthropic"
    openai_api_key: str = ""
    openai_base_url: str = "https://api.openai.com/v1"
    openai_model: str = "gpt-4o"
    tmdb_api_key: str = DEFAULT_TMDB_API_KEY
    tmdb_language: str = "zh-CN"  # TMDB metadata language (zh-CN, en-US, ja-JP, etc.)

    @field_validator("tmdb_api_key", mode="before")
    @classmethod
    def _default_tmdb_api_key_when_blank(cls, value: str) -> str:
        if isinstance(value, str) and not value.strip():
            return DEFAULT_TMDB_API_KEY
        return value


class MetadataParserConfig(BaseModel):
    provider: str = "regex"


class MetadataValidatorConfig(BaseModel):
    provider: str = "tmdb"


class BotConfig(BaseModel):
    """Configuration for a single notification bot."""

    type: str  # "telegram" or "pushplus"
    enabled: bool = True
    config: dict[str, Any] = Field(default_factory=dict)


class NotificationConfig(BaseModel):
    """Configuration for notification system."""

    enabled: bool = False
    batch_interval: float = (
        300.0  # Batch notifications interval in seconds (default: 5 minutes, 0 to disable)
    )
    bots: list[BotConfig] = Field(default_factory=list)


class TelegramAssistantConfig(BaseModel):
    """Configuration for Telegram assistant bot."""

    enabled: bool = False
    bot_token: str = ""
    allowed_users: list[int] = Field(default_factory=list)


class WechatAssistantConfig(BaseModel):
    """Configuration for WeChat/iLink assistant bot."""

    enabled: bool = False
    account_id: str = ""
    token: str = ""
    base_url: str = "https://ilinkai.weixin.qq.com"
    home_channel: str = ""
    allowed_users: list[str] = Field(default_factory=list)
    dm_policy: str = "open"


class FeishuAssistantConfig(BaseModel):
    """Configuration for Feishu/Lark assistant bot."""

    enabled: bool = False
    app_id: str = ""
    app_secret: str = ""
    domain: str = "feishu"
    connection_mode: str = "websocket"
    webhook_host: str = "127.0.0.1"
    webhook_port: int = 8765
    webhook_path: str = "/feishu/webhook"
    bot_open_id: str = ""
    require_mention: bool = True
    state_dir: str = "data/messaging"
    allowed_users: list[str] = Field(default_factory=list)


class AutoDreamConfig(BaseModel):
    """Configuration for auto-dream memory consolidation."""

    enabled: bool = True
    min_hours: float = 24.0  # Minimum hours since last consolidation
    min_sessions: int = 5  # Minimum sessions since last consolidation


class AssistantConfig(BaseModel):
    """Configuration for assistant module."""

    enabled: bool = False
    max_context_tokens: int = 128_000
    session_compact_threshold: int = 100_000
    skills_dir: str = "skills"  # Skill search directory
    data_dir: str = "data/assistant"  # Memory file directory
    telegram: TelegramAssistantConfig = Field(default_factory=TelegramAssistantConfig)
    wechat: WechatAssistantConfig = Field(default_factory=WechatAssistantConfig)
    feishu: FeishuAssistantConfig = Field(default_factory=FeishuAssistantConfig)
    auto_dream: AutoDreamConfig = Field(default_factory=AutoDreamConfig)


class LogConfig(BaseModel):
    """Configuration for logging."""

    level: str = "INFO"  # Log level: DEBUG, INFO, WARNING, ERROR, FATAL
    rotation: str = (
        "00:00"  # Log rotation time (e.g., "00:00" for midnight, "500 MB" for size-based)
    )
    retention: str = "1 week"  # How long to keep old logs


class BangumiConfig(BaseModel):
    """Configuration for Bangumi API integration."""

    access_token: str = (
        ""  # Bangumi API Access Token (also supports env var BANGUMI_TOKEN)
    )


class MikanConfig(BaseModel):
    """Configuration for Mikan (mikanani.me) integration."""

    username: str = ""  # Mikan account username
    password: str = ""  # Mikan account password


class ProxyConfig(BaseModel):
    """Configuration for proxy settings."""

    http: str = ""  # HTTP proxy URL (e.g., "http://127.0.0.1:7890")
    https: str = ""  # HTTPS proxy URL (e.g., "http://127.0.0.1:7890")


class BackendConfig(BaseModel):
    """Configuration for the backend API server."""

    host: str = "127.0.0.1"  # Bind address (localhost only by default)
    port: int = 26666  # Listening port


class UserConfig(BaseModel):
    downloader: DownloaderConfig = DownloaderConfig()
    file_renamer: FileRenamerConfig = FileRenamerConfig()
    metadata_parser: MetadataParserConfig = MetadataParserConfig()
    metadata_validator: MetadataValidatorConfig = MetadataValidatorConfig()
    rss: RSSConfig = RSSConfig()
    openlist: OpenListConfig = OpenListConfig()
    llm: LLMConfig = LLMConfig()
    notification: NotificationConfig = NotificationConfig()
    assistant: AssistantConfig = AssistantConfig()
    log: LogConfig = LogConfig()
    proxy: ProxyConfig = ProxyConfig()
    bangumi: BangumiConfig = BangumiConfig()
    mikan: MikanConfig = MikanConfig()
    backend: BackendConfig = BackendConfig()

    @model_validator(mode="before")
    @classmethod
    def _prefer_llm_parser_when_llm_key_is_configured(cls, values: Any) -> Any:
        """Keep regex as the default, but prefer LLM when a key is configured.

        Explicit ``metadata_parser.provider`` always wins so users can keep
        regex parsing while using LLM for other modules, such as the assistant.
        """
        if not isinstance(values, dict):
            return values

        metadata_parser = values.get("metadata_parser")
        if cls._has_explicit_metadata_parser_provider(metadata_parser):
            return values

        if not cls._has_llm_api_key(values.get("llm")):
            return values

        updated_values = dict(values)
        parser_config = (
            dict(metadata_parser) if isinstance(metadata_parser, dict) else {}
        )
        parser_config["provider"] = "llm"
        updated_values["metadata_parser"] = parser_config
        return updated_values

    @staticmethod
    def _has_explicit_metadata_parser_provider(metadata_parser: Any) -> bool:
        if isinstance(metadata_parser, dict):
            provider = metadata_parser.get("provider")
        else:
            provider = getattr(metadata_parser, "provider", None)
        return isinstance(provider, str) and bool(provider.strip())

    @staticmethod
    def _has_llm_api_key(llm_config: Any) -> bool:
        if isinstance(llm_config, dict):
            api_key = llm_config.get("openai_api_key")
        else:
            api_key = getattr(llm_config, "openai_api_key", None)
        return isinstance(api_key, str) and bool(api_key.strip())


class ConfigManager:
    def __init__(self, config_path: str = "config.toml"):
        self.config_path = Path(os.getcwd()) / config_path
        self._config: UserConfig = UserConfig()
        self._load_failed: bool = False

        self._load_from_file()

    def _set_proxy_env(self) -> None:
        """Set proxy environment variables from configuration."""
        from .environment import ProxyEnvironmentApplier

        ProxyEnvironmentApplier().apply(self._config.proxy)

    def _load_from_file(self) -> None:
        """Load configuration from file during adapter construction."""
        if not self.config_path.exists():
            self.save()
            return

        try:
            content = self.config_path.read_bytes()
            raw = tomllib.loads(content.decode("utf-8"))
            self._config = UserConfig.model_validate(raw)
            if self._remove_placeholder_rss(save=False):
                self.save()
            self._load_failed = False
            self._set_proxy_env()
        except Exception as e:
            self._load_failed = True
            logger.log(
                FATAL_LEVEL,
                f"Failed to load configuration from {self.config_path}: {e}. "
                "Application will exit.",
            )

    @property
    def data(self) -> UserConfig:
        """Get the in-memory configuration snapshot."""
        return self._config

    def save(self) -> None:
        """Save current configuration to file."""
        try:
            # TOML has no null literal.  Optional subscription metadata such as
            # an unresolved TMDB id must be omitted until it is available.
            payload = self._config.model_dump(exclude_none=True)
            self.config_path.write_text(toml_dumps(payload), encoding="utf-8")
        except Exception as e:
            logger.error(
                f"Failed to save configuration to {self.config_path}: {e}. "
                "Runtime changes may not persist after restart."
            )

    def add_rss_url(
        self,
        url: str,
        *,
        name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
        exclude_patterns: list[str] | None = None,
    ) -> None:
        """Add or reactivate an RSS subscription."""
        self._remove_placeholder_rss(save=False)
        existing = next(
            (item for item in self._config.rss.subscriptions if item.url == url), None
        )
        if existing is None:
            self._config.rss.subscriptions.append(
                RSSSubscription(
                    url=url,
                    name=name.strip(),
                    tmdb_id=tmdb_id,
                    poster_url=poster_url.strip(),
                    exclude_patterns=list(exclude_patterns or []),
                )
            )
        else:
            existing.enabled = True
            if name.strip():
                existing.name = name.strip()
            if tmdb_id is not None:
                existing.tmdb_id = tmdb_id
            if poster_url.strip():
                existing.poster_url = poster_url.strip()
            if exclude_patterns is not None:
                existing.exclude_patterns = list(exclude_patterns)
        self._sync_active_rss_urls()
        self.save()

    def update_rss_subscription(
        self,
        url: str,
        *,
        name: str | None = None,
        enabled: bool | None = None,
        tmdb_id: int | None = None,
        poster_url: str | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> bool:
        """Update display metadata or pause/resume a subscription."""
        item = next(
            (item for item in self._config.rss.subscriptions if item.url == url), None
        )
        if item is None:
            return False
        if name is not None and name.strip():
            item.name = name.strip()
        if enabled is not None:
            item.enabled = enabled
        if tmdb_id is not None:
            item.tmdb_id = tmdb_id
        if poster_url is not None:
            item.poster_url = poster_url.strip()
        if exclude_patterns is not None:
            item.exclude_patterns = list(exclude_patterns)
        self._sync_active_rss_urls()
        self.save()
        return True

    def update_rss_filter(self, *, exclude_patterns: list[str]) -> None:
        """Update the global RSS title exclusion list and persist it."""
        self._config.rss.filter.exclude_patterns = list(exclude_patterns)
        self.save()

    def update_llm_settings(
        self,
        *,
        provider_type: str | None = None,
        api_key: str | None = None,
        base_url: str | None = None,
        model: str | None = None,
        tmdb_language: str | None = None,
        metadata_parser_provider: str | None = None,
    ) -> None:
        """Persist the LLM/metadata settings edited by the built-in UI.

        An omitted API key keeps the existing secret, so opening the settings
        page never requires returning the key to the browser.
        """
        if provider_type is not None and provider_type.strip():
            self._config.llm.provider_type = provider_type.strip()
        if api_key is not None and api_key.strip():
            self._config.llm.openai_api_key = api_key.strip()
        if base_url is not None and base_url.strip():
            self._config.llm.openai_base_url = base_url.strip().rstrip("/")
        if model is not None and model.strip():
            self._config.llm.openai_model = model.strip()
        if tmdb_language is not None and tmdb_language.strip():
            self._config.llm.tmdb_language = tmdb_language.strip()
        if metadata_parser_provider is not None and metadata_parser_provider.strip():
            self._config.metadata_parser.provider = metadata_parser_provider.strip()
        self.save()

    def update_openlist_settings(
        self,
        *,
        download_path: str | None = None,
        rename_format: str | None = None,
    ) -> None:
        """Persist the validated OpenList destination and rename format."""
        if download_path is not None:
            self._config.openlist.download_path = download_path.strip()
        if rename_format is not None:
            self._config.openlist.rename_format = rename_format.strip()
        self.save()

    def remove_rss_url(self, url: str) -> bool:
        """Remove an RSS URL from configuration."""
        before = len(self._config.rss.subscriptions)
        self._config.rss.subscriptions = [
            item for item in self._config.rss.subscriptions if item.url != url
        ]
        if len(self._config.rss.subscriptions) == before:
            return False
        self._sync_active_rss_urls()
        self.save()
        return True

    def _sync_active_rss_urls(self) -> None:
        self._config.rss.urls = [
            item.url for item in self._config.rss.subscriptions if item.enabled
        ]

    def _remove_placeholder_rss(self, *, save: bool) -> bool:
        before = len(self._config.rss.subscriptions)
        self._config.rss.subscriptions = [
            item
            for item in self._config.rss.subscriptions
            if item.url != PLACEHOLDER_RSS_URL
        ]
        self._config.rss.urls = [
            item.url
            for item in self._config.rss.subscriptions
            if item.enabled
        ]
        changed = len(self._config.rss.subscriptions) != before
        if changed and save:
            self.save()
        return changed

    @property
    def rss(self) -> RSSConfig:
        return self.data.rss

    @property
    def downloader(self) -> DownloaderConfig:
        return self.data.downloader

    @property
    def file_renamer(self) -> FileRenamerConfig:
        return self.data.file_renamer

    @property
    def openlist(self) -> OpenListConfig:
        return self.data.openlist

    @property
    def llm(self) -> LLMConfig:
        return self.data.llm

    @property
    def metadata_parser(self) -> MetadataParserConfig:
        return self.data.metadata_parser

    @property
    def metadata_validator(self) -> MetadataValidatorConfig:
        return self.data.metadata_validator

    @property
    def notification(self) -> NotificationConfig:
        return self.data.notification

    @property
    def log(self) -> LogConfig:
        return self.data.log

    @property
    def assistant(self) -> AssistantConfig:
        return self.data.assistant

    @property
    def proxy(self) -> ProxyConfig:
        return self.data.proxy

    @property
    def bangumi(self) -> BangumiConfig:
        return self.data.bangumi

    @property
    def bangumi_token(self) -> str:
        """Get Bangumi token with env var override."""
        return os.environ.get("BANGUMI_TOKEN", "") or self.bangumi.access_token

    @property
    def mikan(self) -> MikanConfig:
        return self.data.mikan

    @property
    def backend(self) -> BackendConfig:
        return self.data.backend

    @property
    def backend_url(self) -> str:
        """Get the full backend API base URL."""
        local_backend_scheme = "http"
        return f"{local_backend_scheme}://{self.backend.host}:{self.backend.port}"

    @property
    def load_failed(self) -> bool:
        return self._load_failed


_config_instance: ConfigManager | None = None


def load_config(config_path: str | None = None) -> ConfigManager:
    """Load configuration explicitly from *config_path* or CONFIG_PATH."""
    return ConfigManager(config_path or os.environ.get("CONFIG_PATH", "config.toml"))


def get_config() -> ConfigManager:
    """Return the process configuration, loading it on first use."""
    global _config_instance
    if _config_instance is None:
        _config_instance = load_config()
    return _config_instance


class LazyConfig:
    """Lazy proxy for the process configuration.

    Importing this module should not read/write config files or mutate the
    process environment. The first attribute access performs the load.
    """

    def __getattr__(self, name: str) -> Any:
        return getattr(get_config(), name)

    def __repr__(self) -> str:
        status = "loaded" if _config_instance is not None else "unloaded"
        return f"<LazyConfig {status}>"


config = LazyConfig()
