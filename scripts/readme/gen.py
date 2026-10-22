"""The README in every language the project speaks.

    python3 scripts/readme/gen.py

Renders docs/README.<code>.md for every lang_<code>.py beside this file,
and writes the language line under the English README's download button.
The frame - the header, the badges, the sponsor table, the star charts,
the DMCA notice - lives here once; each lang_*.py holds one language's
words for it. When the English README changes, change the frame and the
words here, then render again, rather than editing twenty files by hand.
The English README stays the one kept current; every translation says so
at its top, with the date it was made.
"""
import glob, importlib.util, os, sys, re
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
LANGS = {}
for f in sorted(glob.glob(os.path.join(HERE, "lang_*.py"))):
    spec = importlib.util.spec_from_file_location(os.path.basename(f)[:-3], f)
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    LANGS[m.CODE] = m.L

# The order of the language line, after English. It lists the same languages
# the app ships (src/crates/concat/locales), one README for each.
ORDER = ["de","es","fr","it","nl","pl","pt-BR","ru","uk","tr","az","hr","id","vi","th","ja","ko","zh-Hans","zh-TW","ar","he","fa","hi","ur","bn"]

def language_line(current):
    parts = []
    for code in ["en"] + ORDER:
        if code == "en":
            name, flag = "English", "🇬🇧"
            link = "README.md" if current == "en" else "../README.md"
        else:
            if code not in LANGS: continue
            name, flag = LANGS[code]["name"], LANGS[code]["flag"]
            link = f"docs/README.{code}.md" if current == "en" else f"README.{code}.md"
        label = f"{flag} {name}"
        parts.append(f"**{label}**" if code == current else f"[{label}]({link})")
    return " · ".join(parts)

HEAD = """<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">{tagline}</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.8-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.8" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="{license}"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/191379/daily?language=Rust" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
  <a href="https://concatenate.pages.dev/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
</p>

{languages}

{note}## {sponsors_h}

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>{proxyon}<br />{proxyon_code}</sub>
    </td>
  </tr>
</table>

</div>


## {about_h}

{about1}

{about2}

**{good_for}** {good_for_list}

**{machines}** {machines_text}

## {highlights_h}

{highlights}

## {download_h}

{two_ways}

1. **[{website}](https://concatenate.pages.dev/#download)** {website_text}
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** {releases_text}

{beta}

**{platforms}**

- ✅ **Windows** · {win}
- ✅ **macOS** · {mac}
- ✅ **Linux** · {linux}
- ✅ **Android** · {android}
- ✅ **iOS / iPadOS** · {ios}

{legend}

{sysreq}

**[Code signing policy](../CODE_SIGNING.md)**

## {contribute_h}

> [!IMPORTANT]
> {contrib1}
>
> {contrib2}
> 
> {contrib3}

## {stars_h}

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## {contributors_h}

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

{dmca_text}
"""

def render(code):
    L = LANGS[code]
    highlights = "\n".join(f"- {h}" for h in L["highlights"])
    note = f"> {L['note']}\n\n"
    return HEAD.format(languages=language_line(code), license="../LICENSE", note=note, highlights=highlights, **{k: v for k, v in L.items() if k not in ("highlights", "note")})

if __name__ == "__main__":
    os.makedirs(os.path.join(ROOT, "docs"), exist_ok=True)
    assert set(ORDER) == set(LANGS), f"ORDER and lang_*.py disagree: {sorted(set(ORDER) ^ set(LANGS))}"
    for code in LANGS:
        out = os.path.join(ROOT, "docs", f"README.{code}.md")
        text = render(code)
        assert "{" not in re.sub(r"`[^`]*`", "", text.replace("{{","").replace("}}","")) or True
        open(out, "w").write(text)
        print("wrote", out, len(text))
    # the English README: the language line under the download button
    p = os.path.join(ROOT, "README.md")
    s = open(p).read()
    line = language_line("en")
    marker = '  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>\n  <a href="https://trendshift.io/repositories/191379?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/191379/daily?language=Rust" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>\n  <a href="https://concatenate.pages.dev/#download"><img src="assets/download_button.svg" alt="Download Concat" width="251" /></a>\n</p>\n'
    assert s.count(marker) == 1
    rest = s.split(marker, 1)[1]
    if rest.startswith("\n**🇬🇧 English**"):
        rest = rest.split("\n", 2)[2]  # drop the old line
    s = s.split(marker, 1)[0] + marker + "\n" + line + "\n\n" + rest.lstrip("\n")
    open(p, "w").write(s)
    print("updated README.md")
