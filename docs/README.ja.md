<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">無料でオープンソース、クロスプラットフォームの動画エディタ。</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.12-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.12" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/191379" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · **🇯🇵 日本語** · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> 2026 年 10 月 11 日時点の英語版 README からの翻訳です。最新の内容は英語版が保持します。

## 有料スポンサー

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>開発者向けの従量課金プロキシ<br />コード <code>JUB0T</code> で 20 % オフ</sub>
    </td>
  </tr>
</table>

</div>


## 概要

Concat は macOS・Windows・Linux・Android・iOS 向けの、無料でオープンソースの動画エディタです。たいていの編集に必要なものをカバーします。自動字幕、声の分離、テキスト読み上げ、背景除去、アニメーションとキーフレーム、エフェクトとタイトル、マルチトラック編集、4K 書き出し。しかも落とし穴なし。透かしなし、アカウント不要、サブスクリプション不要、アップロードなし。

すべてが GPU コンポジターを備えたネイティブ Rust エンジン上でローカルに動きます。インストールして、素材を放り込んで、切る。字幕・音声・切り抜き用の AI モデルは設定から一度だけダウンロードし、以降はオフラインで動作します。あなたの素材がディスクの外へ出ることはありません。

**向いている用途:** TikTok・Reels・Shorts、YouTube 動画、チュートリアルや画面収録、ポッドキャストの切り抜き、ミーム。

**機械にも:** JSON-RPC の API があるので、スクリプトや AI エージェントも動画を編集できます。

## 特長

- 🚫 **透かしなし。アカウント不要。有料の壁なし。** 永久に。
- 🔒 **100 % ローカル。** アップロードは一切なし、AI も同じです。オフラインで動作。
- 💬 **自動字幕。** 話している言語で、お使いのマシン上で生成され、スタイル付きでそのままタイムラインに入ります。
- 🎤 **声の分離。** 音楽やノイズから声を取り出す、あるいは声だけを取り除く。それぞれを別のレベルで調整できます。
- 🗣️ **テキスト読み上げ + ボイスクローン。** 無料のローカル音声、または数秒の録音から作る好きな声。デスクトップで使えます。
- 🧍 **背景除去、クロマキー、マスク。** 人物や物体を自動で切り抜き、グリーンバックは自動でキー抜き、さらにクリップごとにマスクを重ねられ、それぞれに専用のモードとキーフレームがあります。
- 🌀 **イン・アウト・ループのアニメーション。** クリップ、画像、タイトル、図形に使えます。選ぶ前に、それぞれのカード上で再生されます。
- ✨ **100 以上のエフェクト、フィルター、トランジションと 25 のサウンドエフェクト。** シネマ調のルック、シェイクとインパクト、レトロ、ワイプまで、すべてプレビューでそのまま確認できます。トランジションはほかのクリップと同じように動かしてトリムできるクリップです。
- 🎞️ **キーフレーム。** 位置、スケール、サイズ、回転、不透明度、音量、エフェクトのパラメーター。カーブエディターも内蔵。
- 🏃 **スピードランプとカーブ。** 一定でも描いたものでも、なめらかでも直線でも。フリーズフレームと逆再生にも対応。
- 🪄 **高画質化と手ぶれ補正。** ぼやけた映像や古い映像を 2K まで復元。揺れたショットも安定させます。
- ✂️ **速く切って、正確に切る。** 分割、トリム、リップル、結合、連動する字幕とオーバーレイ、お好みでマグネティックタイムライン。どのカットもフレーム単位で決まり、プレビューと書き出しで同じです。
- 🎚️ **マルチトラック、マルチタイムライン。** 動画、静止画、タイトル、図形をいくつでもトラックに重ねられます。1 つのプロジェクトに複数のカット。描画モード、クロップ、反転。
- 📝 **タイトルと図形。** フォント、縁取り、影、背景プレート、カラー絵文字、アラビア語・中国語・ヒンディー語・タイ語ほか 8 言語向けのプリセット。グラデーション、星形、アウトライン、影を使える図形。
- 🎙️ **ナレーションとワンスイッチの音声クリーンアップ。** カットに合わせて録音。ノイズ除去、声の強調、音量の均一化。さらにチップマンク、ロボット、電話などの声も。
- 📤 **書き出し。** H.264、HEVC、AV1。お使いのコンピューター自身のエンコーダーで。最大 4K 60、10 ビットカラー。
- ⚡ **高速。** プロキシ内蔵で、4K もなめらかにスクラブできます。タイトルは全コアで描画され、ウィンドウが開いた瞬間にすべてのエフェクトが使えます。
- 🛟 **どんなマシンでも動き続ける。** グラフィックチップが力尽きても、プレビューは描画を続けます。
- 🤖 **スクリプト対応。** スクリプトや AI エージェントが動画を編集できる JSON-RPC の API。ソースからビルドすると gRPC とコマンドラインツールも使えます。
- 🖥️ **macOS、Windows、Linux、Android、iOS。** 27 言語。同じアプリ、同じプロジェクトファイル。

## ダウンロード

入口は 2 つ:

1. **[ウェブサイト](https://concat.standalones.org/#download)** があなたのマシンに合ったビルドを渡します。まずはここから。
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** にはすべてのプラットフォーム向けのすべてのビルドが、インストーラー、パッケージ、チェックサムとともにあります。自分で選びたいときに。

Concat は **ベータ版** です。動きますが、まだ粗い部分があります。見つけたら [教えてください](https://github.com/jub0t/Concat/issues)。

**プラットフォーム**

- ✅ **Windows** · x86_64 (ARM ビルドは一時停止中、[#326](https://github.com/jub0t/Concat/issues/326) を参照)。セットアップと `.msi`。SmartScreen が未署名ビルドを止めたら: **詳細情報** › **実行**
- ✅ **macOS** · Intel と Apple シリコン。macOS が未署名ビルドを開かないときは: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 と ARM。`.deb`、`.rpm`、`.AppImage`、Arch パッケージ。さらに **[Flatpark](https://flatpark.org/apps/app.concat.editor/)** 上の Flatpak も。各リリースの x86_64 `.deb` を包み、リリースに合わせて更新されるコミュニティ運営の Flatpak リモートです
- ✅ **Android** · スマートフォンとタブレット
- ✅ **iOS / iPadOS** · iPhone と iPad、サイドロード

✅ 対応 · 🚧 作業中 · 🧪 検証待ち

**システム要件** と任意モデルのサイズは [ウェブサイト](https://concat.standalones.org/guides/system-requirements) にあります。

**[Code signing policy](../CODE_SIGNING.md)**

## 貢献するには

> [!IMPORTANT]
> いちばんの貢献は、[Releases](https://github.com/jub0t/Concat/releases) ページからビルドを取って使うことです。どこで壊れるかを見つけ、どこがもっと良くなるかを伝えてください。
>
> コードを書く準備ができたら、[CONTRIBUTING.md](../CONTRIBUTING.md) にセットアップ、ツリーの構成、実行すべきチェック、貢献のライセンスがまとまっています。スクリプトやサービス、エージェントから Concat を動かすなら、[開発者向けドキュメント](https://concat.standalones.org/docs) に Concat API とそのトランスポート (JSON-RPC、gRPC) があります。プロジェクトは [この Discussion](https://github.com/jub0t/Concat/discussions/3) で発表されました。
> 
> 貢献者は Discord サーバーで `@Contributor` ロールを受け取れます。声をかけてください。

## スターの推移

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## 貢献者

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

このリポジトリ、そのリリース、またはここで配布しているパッケージやモデルがあなたの著作権を侵害していると思われる場合は、次の内容を添えて [jub0trd@gmail.com](mailto:jub0trd@gmail.com) までメールでご連絡ください: あなたが権利を持つ著作物の説明、侵害している素材がここに掲載されている場所へのリンク、あなたの連絡先、その利用があなた、あなたの代理人、または法律によって許諾されていないと誠実に信じている旨の声明、そして偽証罪の罰則のもとで、通知の内容が正確であり、あなたが権利者本人または権利者に代わって行動する権限を持つ者である旨の声明（あなたの手書きの署名または電子署名を付したもの）。有効な通知には速やかに対応します。また、素材を削除された方は誰でも、同じアドレスに異議申し立て通知（カウンター通知）を送ることができます。
