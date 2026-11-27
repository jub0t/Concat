<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">Trình biên tập video miễn phí, mã nguồn mở và đa nền tảng.</h3>

<div align="center">

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.13-0568FD?style=flat&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.13" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
</p>

<p align="center">
  <a href="https://www.star-history.com/jub0t/concat"><img src="https://api.star-history.com/badge?repo=jub0t/concat&type=rank" alt="Star History Rank" height="62" /></a>
  <a href="https://concat.standalones.org/#download"><img src="../assets/download_button.svg" alt="Download Concat" width="251" /></a>
  <a href="https://trendshift.io/repositories/191379?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-191379" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/191379" alt="jub0t%2FConcat | Trendshift" width="250" height="55"/></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · [🇧🇷 Português (Brasil)](README.pt-BR.md) · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · **🇻🇳 Tiếng Việt** · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Dịch từ README tiếng Anh ngày 11 tháng 10 năm 2026. Bản tiếng Anh là bản được cập nhật thường xuyên.

## Nhà tài trợ trả phí

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxy trả theo mức dùng cho lập trình viên<br />Dùng mã <code>JUB0T</code> để giảm 20 %</sub>
    </td>
  </tr>
</table>

</div>


## Giới thiệu

Concat là trình biên tập video miễn phí, mã nguồn mở cho macOS, Windows, Linux và Android. Nó có những gì phần lớn các bản dựng cần: phụ đề tự động, tách giọng, chuyển văn bản thành giọng nói, xóa nền, hoạt ảnh và keyframe, hiệu ứng và tiêu đề, cắt nhiều track, xuất 4K. Không có bất kỳ ràng buộc nào: không watermark, không tài khoản, không đăng ký, không tải lên.

Mọi thứ chạy cục bộ trên engine Rust gốc với trình tổng hợp GPU. Cài đặt, kéo thả footage, cắt. Các mô hình AI cho phụ đề, giọng nói và tách nền tải một lần từ Cài đặt và sau đó hoạt động ngoại tuyến. Footage của bạn không bao giờ rời khỏi ổ đĩa của bạn.

**Phù hợp cho:** TikTok, Reels và Shorts, video YouTube, hướng dẫn và ghi màn hình, trích đoạn podcast, meme.

**Cũng dành cho máy móc:** API JSON-RPC, để script và các tác nhân AI cũng có thể cắt video bằng nó.

## Điểm nổi bật

- 🚫 **Không watermark. Không tài khoản. Không tường phí.** Mãi mãi.
- 🔒 **100% cục bộ.** Không có gì được tải lên, kể cả AI. Hoạt động ngoại tuyến.
- 💬 **Phụ đề tự động.** Bằng ngôn ngữ bạn nói, tạo ngay trên máy bạn, đã có kiểu dáng và nằm sẵn trên timeline.
- 🎤 **Tách giọng.** Tách giọng nói khỏi nhạc hoặc tiếng ồn, hoặc bỏ giọng đi, mỗi phần có mức âm lượng riêng.
- 🗣️ **Chuyển văn bản thành giọng nói + nhân bản giọng.** Một giọng cục bộ miễn phí, hoặc bất kỳ giọng nào từ vài giây ghi âm. Trên máy tính.
- 🧍 **Xóa nền, chroma key và mặt nạ.** Người và vật thể được cắt tách giúp bạn, phông xanh tự tách, và một chồng mặt nạ trên mỗi clip, mỗi mặt nạ có chế độ và keyframe riêng.
- 🌀 **Hoạt ảnh Vào, Ra và Lặp.** Cho clip, ảnh, tiêu đề và hình khối. Mỗi hoạt ảnh chạy ngay trên thẻ của nó trước khi bạn chọn.
- ✨ **Hơn 100 hiệu ứng, bộ lọc và chuyển cảnh, 25 hiệu ứng âm thanh.** Phong cách điện ảnh, rung lắc và va chạm, hoài cổ, wipe, tất cả xem trực tiếp trong bản xem trước. Chuyển cảnh là một clip bạn di chuyển và cắt tỉa như mọi clip khác.
- 🎞️ **Keyframe.** Vị trí, tỷ lệ, kích thước, xoay, độ mờ, âm lượng, tham số hiệu ứng. Có sẵn trình chỉnh đường cong.
- 🏃 **Đường tăng tốc và đường cong tốc độ.** Hằng số hoặc tự vẽ, mượt hoặc thẳng. Đóng băng khung hình và chạy ngược.
- 🪄 **Tăng cường và Chống rung.** Phục hồi cảnh quay mềm hoặc cũ, tới 2K. Làm dịu một cảnh quay rung.
- ✂️ **Cắt nhanh, cắt chuẩn.** Tách, tỉa, ripple, gộp, phụ đề và lớp phủ liên kết, timeline từ tính nếu bạn muốn. Mỗi nhát cắt rơi đúng vào khung hình, giống hệt trong bản xem trước và khi xuất.
- 🎚️ **Nhiều track, nhiều timeline.** Xếp video, ảnh tĩnh, tiêu đề và hình khối lên bao nhiêu track tùy thích. Nhiều bản dựng trong một dự án. Chế độ hòa trộn, cắt khung, lật.
- 📝 **Tiêu đề và hình khối.** Phông chữ, viền, đổ bóng, nền chữ, emoji màu và mẫu có sẵn cho tiếng Ả Rập, Trung, Hindi, Thái và tám ngôn ngữ khác. Hình khối có chuyển màu, ngôi sao, đường viền và bóng đổ.
- 🎙️ **Thuyết minh và làm sạch giọng chỉ với một công tắc.** Thu âm đè lên bản dựng của bạn. Khử ồn, tăng cường giọng, cân bằng âm lượng. Cùng giọng sóc chuột, robot, điện thoại và nhiều kiểu khác.
- 📤 **Xuất.** H.264, HEVC, AV1, bằng bộ mã hóa riêng của máy tính. Tới 4K 60, màu 10-bit.
- ⚡ **Nhanh.** Có sẵn proxy, nên 4K tua mượt. Tiêu đề được vẽ trên mọi nhân CPU, và mọi hiệu ứng sẵn sàng ngay khi cửa sổ mở.
- 🛟 **Chạy tiếp trên mọi máy.** Khi chip đồ họa bỏ cuộc, bản xem trước vẫn tiếp tục vẽ.
- 🤖 **Có thể lập trình.** API JSON-RPC để script và các tác nhân AI cắt video. Bản dựng từ mã nguồn có thêm công cụ dòng lệnh.
- 🖥️ **macOS, Windows, Linux, Android.** 27 ngôn ngữ. Cùng một ứng dụng, cùng tệp dự án.

## Tải về

Hai cách:

1. **[Trang web](https://concat.standalones.org/#download)** đưa bạn đúng bản dựng cho máy của bạn. Bắt đầu từ đây.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** có mọi bản dựng cho mọi nền tảng, kèm trình cài đặt, gói và checksum. Khi bạn muốn tự chọn.

Concat đang ở giai đoạn **beta**: nó hoạt động, và vẫn còn vài góc cạnh. [Hãy cho biết](https://github.com/jub0t/Concat/issues) khi bạn gặp một lỗi.

**Nền tảng**

- ✅ **Windows** · x86_64 (bản dựng ARM đang tạm dừng, xem [#326](https://github.com/jub0t/Concat/issues/326)). Một bộ cài và một `.msi`. Nếu SmartScreen chặn bản dựng chưa ký: **More info** › **Run anyway**
- ✅ **macOS** · Intel và Apple Silicon. Nếu macOS từ chối mở bản dựng chưa ký: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 và ARM. `.deb`, `.rpm`, `.AppImage` và gói Arch. Ngoài ra còn Flatpak trên **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, một kho Flatpak cộng đồng đóng gói `.deb` x86_64 của mỗi bản phát hành và cập nhật theo nó
- ✅ **Android** · điện thoại và máy tính bảng

✅ Được hỗ trợ · 🚧 Đang thực hiện · 🧪 Cần kiểm thử

**Yêu cầu hệ thống** và kích thước các mô hình tùy chọn có trên [trang web](https://concat.standalones.org/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Cách đóng góp

> [!IMPORTANT]
> Cách đóng góp tốt nhất là lấy một bản dựng từ trang [Releases](https://github.com/jub0t/Concat/releases) và dùng nó: tìm chỗ nó hỏng, và nói chỗ nó có thể tốt hơn.
>
> Sẵn sàng viết mã? [CONTRIBUTING.md](../CONTRIBUTING.md) mô tả cách thiết lập, bố cục cây mã nguồn, các kiểm tra cần chạy và cách cấp phép cho đóng góp. Điều khiển Concat từ script, dịch vụ hay tác nhân? [Tài liệu dành cho nhà phát triển](https://concat.standalones.org/docs) mô tả API của Concat và phương thức truyền tải: JSON-RPC. [Thảo luận này](https://github.com/jub0t/Concat/discussions/3) là nơi dự án được công bố.
> 
> Người đóng góp có thể nhận vai trò `@Contributor` trên máy chủ Discord: chỉ cần hỏi.

## Lịch sử sao

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Người đóng góp

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Nếu bạn cho rằng nội dung nào đó trong kho mã này, trong các bản phát hành của nó, hoặc trong một gói hay mô hình mà nó phân phối vi phạm bản quyền của bạn, hãy gửi email tới [jub0trd@gmail.com](mailto:jub0trd@gmail.com) kèm theo: mô tả tác phẩm thuộc sở hữu của bạn, liên kết tới nơi tài liệu vi phạm xuất hiện tại đây, thông tin liên hệ của bạn, một tuyên bố rằng bạn tin một cách thiện chí rằng việc sử dụng đó không được bạn, người đại diện của bạn hoặc pháp luật cho phép, và một tuyên bố, chịu hình phạt về tội khai man, rằng thông báo là chính xác và bạn là chủ sở hữu hoặc được ủy quyền hành động thay mặt chủ sở hữu, có chữ ký tay hoặc chữ ký điện tử của bạn. Các thông báo hợp lệ sẽ được xử lý kịp thời, và bất kỳ ai có tài liệu bị gỡ bỏ đều có thể gửi thông báo phản đối (counter-notice) tới cùng địa chỉ này.
