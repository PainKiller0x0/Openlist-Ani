from openlist_ani.assistant.skill_support.mikan_client import MikanClient
from openlist_ani.adapters.outbound.feed_sources.factory import FeedSourceFactory
from openlist_ani.adapters.outbound.feed_sources.mikan import MikanFeedSource
from openlist_ani.adapters.outbound.configuration.settings import MikanConfig


def test_mikan_config_defaults_to_current_site():
    assert MikanConfig().base_url == "https://mikanani.kas.pub/"


def test_mikan_rss_url_uses_configured_site_and_group():
    client = MikanClient(
        username="",
        password="",
        base_url="https://mikanani.kas.pub/",
    )

    assert (
        client.rss_url(3992, 359)
        == "https://mikanani.kas.pub/RSS/Bangumi?bangumiId=3992&subgroupid=359"
    )
    assert (
        client.rss_url(3992)
        == "https://mikanani.kas.pub/RSS/Bangumi?bangumiId=3992"
    )


def test_mikan_search_parser_keeps_custom_site_base_url():
    html = '<a href="/Home/Bangumi/3992">尼古喵喵</a>'

    result = MikanClient._parse_search_results(html, "https://mikan.example/")

    assert result == [
        {
            "bangumi_id": 3992,
            "name": "尼古喵喵",
            "url": "https://mikan.example/Home/Bangumi/3992",
        }
    ]


def test_mikan_subgroup_parser_returns_group_and_release():
    html = """
    <a class="subgroup-name" data-anchor="#359">ANi</a>
    <div id="359"></div>
    <div class="episode-table">
      <table><tbody><tr>
        <td><a class="magnet-link-wrap" href="/Home/Episode/abc">[ANi] 尼古喵喵 - 07</a></td>
        <td>2026/08/14</td>
        <td><a class="js-magnet" data-clipboard-text="magnet:?xt=urn:btih:abc"></a></td>
      </tr></tbody></table>
    </div>
    """

    result = MikanClient._parse_subgroups(html, "https://mikan.example")

    assert result[0]["id"] == 359
    assert result[0]["name"] == "ANi"
    assert result[0]["releases"][0]["url"] == (
        "https://mikan.example/Home/Episode/abc"
    )
    assert result[0]["releases"][0]["magnet"] == "magnet:?xt=urn:btih:abc"


def test_configured_mikan_rss_uses_mikan_feed_parser():
    source = FeedSourceFactory().create(
        "https://mikanani.kas.pub/RSS/Bangumi?bangumiId=3992&subgroupid=583"
    )

    assert isinstance(source, MikanFeedSource)
