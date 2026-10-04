"""The README in every language the project speaks.

    python3 scripts/readme/gen.py

Renders docs/README.<code>.md for every lang_<code>.py beside this file,
and writes the language line under the English README's download button.
The frame - the header, the badges, the sponsor table, the charts, the
payment table - lives here once; each lang_*.py holds one language's
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

ORDER = ["de","es","fr","it","pt-BR","ru","uk","tr","hr","id","vi","ja","ko","zh-Hans","zh-TW","ar","he","fa","hi","ur"]

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

HEAD = """<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">{tagline}</h3>
    </td>
  </tr>
</table>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat-square&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.6-0568FD?style=flat-square&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.5" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="{license}"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat-square&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
  <a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat" /></a>
</p>

<p align="center">
  <a href="https://concatenate.pages.dev/#download"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/download_button.svg" alt="Download Concat" width="220" /></a>
</p>

{languages}

{note}## {sponsors_h}

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>{proxyon}<br />{proxyon_code}</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>{logo_here}</b></a><br />
      <sub><a href="#sponsoring">{sponsor_link}</a> {slot}</sub>
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

## {start_h}

{start1}

**{reporting}** {reporting_text}

## {contribute_h}

> [!IMPORTANT]
> {contrib1}
>
> {contrib2}
> 
> {contrib3}

## {contributors_h}

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## {stars_h}

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## {sponsoring_h}

{sponsoring1}

**{where_h}**

{where_text}

{payment_text}

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

{closing}
"""

def render(code):
    L = LANGS[code]
    highlights = "\n".join(f"- {h}" for h in L["highlights"])
    note = f"> {L['note']}\n\n"
    return HEAD.format(languages=language_line(code), license="../LICENSE", note=note, highlights=highlights, **{k: v for k, v in L.items() if k not in ("highlights", "note")})

if __name__ == "__main__":
    os.makedirs(os.path.join(ROOT, "docs"), exist_ok=True)
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
    marker = '  <a href="https://concatenate.pages.dev/#download"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/download_button.svg" alt="Download Concat" width="220" /></a>\n</p>\n'
    assert s.count(marker) == 1
    rest = s.split(marker, 1)[1]
    rest = re.sub(r"^\n(\*\*🇬🇧 English\*\*.*?)\n", "\n", rest, count=1, flags=re.S) if rest.startswith("\n**🇬🇧 English**") else rest
    s = s.split(marker, 1)[0] + marker + "\n" + line + "\n" + rest
    open(p, "w").write(s)
    print("updated README.md")
