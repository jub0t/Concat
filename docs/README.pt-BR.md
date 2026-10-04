<div align="center">
<table width="100%">
  <tr>
    <td align="left" width="120">
      <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark.png" alt="Concat" width="100" />
    </td>
    <td align="">
      <h1>Concat</h1>
      <h3 style="margin-top: -10px;">O substituto do CapCut realmente gratuito, de código aberto e multiplataforma.</h3>
    </td>
  </tr>
</table>

<img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png" alt="Concat editor" width="100%" />

<p align="center">
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/github/downloads/jub0t/concat/total?style=flat-square&logo=github&logoColor=F8F8F8&label=Downloads&labelColor=212123&color=0568FD" alt="Total Downloads" /></a>
  <a href="https://github.com/jub0t/Concat/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/jub0t/Concat/ci.yml?style=flat&logo=githubactions&logoColor=F8F8F8&label=Build&labelColor=000000" alt="Build Status" /></a>
  <a href="https://github.com/jub0t/Concat/releases"><img src="https://img.shields.io/badge/Version-0.2.6-0568FD?style=flat-square&logo=semver&logoColor=F8F8F8&labelColor=212123" alt="Concat Version 0.2.5" /></a>
  <a href="https://discord.gg/DVuPfpXfqP"><img src="https://img.shields.io/badge/Discord-Join%20the%20server-5865F2?style=flat&logo=discord&logoColor=F8F8F8&labelColor=000000" alt="Join Concat Discord" /></a>
  <a href="../LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-0568FD?style=flat-square&logo=gnu&logoColor=F8F8F8&labelColor=212123" alt="License: AGPL-3.0-or-later" /></a>
  <a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat" /></a>
</p>

<p align="center">
  <a href="https://concatenate.pages.dev/#download"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/download_button.svg" alt="Download Concat" width="220" /></a>
</p>

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · **🇧🇷 Português (Brasil)** · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md)

> Traduzido do README em inglês em 2 de outubro de 2026. A versão em inglês é a que se mantém atualizada.

## Patrocinadores pagos

<table width="100%">
  <tr>
    <td align="center" width="50%">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxies pagos por uso para desenvolvedores<br />Use o código <code>JUB0T</code> para 20 % de desconto</sub>
    </td>
    <td align="center" width="50%">
      <a href="#sponsoring"><b>Seu logo aqui</b></a><br />
      <sub><a href="#sponsoring">Patrocine o Concat</a> e seu nome, seu logo e seu link ocupam este espaço</sub>
    </td>
  </tr>
</table>

</div>


## Sobre

O Concat é um editor de vídeo gratuito e de código aberto, uma alternativa ao CapCut para macOS, Windows, Linux e Android. Ele cobre o que as pessoas realmente abrem o CapCut para fazer: legendas automáticas, texto para fala, remoção de fundo, animação por keyframes, efeitos e títulos, corte multipista, exportação em 4K. Sem nenhuma pegadinha: sem marca d’água, sem conta, sem assinatura, sem upload.

Tudo roda localmente em um motor nativo em Rust com um compositor na GPU. Instale, solte o material, corte. Os modelos de IA para legendas, vozes e recorte são baixados uma vez nas Configurações e depois funcionam offline. Seu material nunca sai do seu disco.

**Bom para:** TikTok, Reels e Shorts, vídeos do YouTube, tutoriais e gravações de tela, trechos de podcast, memes.

**Também para máquinas:** uma API JSON-RPC, gRPC e MCP, para que scripts e agentes de IA também possam cortar vídeo com ele.

## Destaques

- 🚫 **Sem marca d’água. Sem conta. Sem paywall.** Nunca.
- 🔒 **100 % local.** Nada é enviado. Funciona offline.
- 💬 **Legendas automáticas.** Whisper local. Escolha o tamanho do modelo e receba legendas estilizadas na linha do tempo.
- 🗣️ **Texto para fala + clonagem de voz.** Vozes locais gratuitas, ou qualquer voz a partir de alguns segundos de gravação.
- 🧍 **Remoção de fundo.** Pessoas, objetos, ou pinte a máscara você mesmo.
- 🎞️ **Keyframes.** Posição, escala, rotação, opacidade, volume, parâmetros de efeitos. Editor de curvas embutido.
- ✨ **Mais de 170 efeitos, filtros, transições e animações de texto.** Renderizados na GPU, ao vivo na prévia.
- ✂️ **Corte rápido.** Dividir, aparar, ripple, mesclar, congelar quadro, velocidade. Linha do tempo magnética se você quiser.
- 🎚️ **Multipista, múltiplas linhas do tempo.** Vários cortes em um projeto. Modos de mesclagem, corte, inversões.
- 📝 **Títulos.** Fontes, contorno, sombra, placa de fundo. Predefinições para começar.
- 🎙️ **Limpeza de voz com um interruptor.** Reduzir ruído, realçar a voz, nivelar o volume. Mais esquilo, robô, telefone e companhia.
- 📤 **Exportação.** H.264, HEVC, AV1. Até 4K 60, cor de 10 bits.
- 🦀 **Motor nativo em Rust.** Compositor na GPU, proxies, decodificação por hardware. O 4K navega suavemente.
- 🤖 **Automatizável.** API JSON-RPC, gRPC e MCP, mais uma CLI. Agentes de IA podem cortar vídeo com ele.
- 🖥️ **macOS, Windows, Linux, Android.** 14 idiomas. O mesmo app, os mesmos arquivos de projeto.

## Download

Dois caminhos:

1. **[O site](https://concatenate.pages.dev/#download)** entrega a build certa para a sua máquina. Comece por aqui.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** tem todas as builds para todas as plataformas, com instaladores, pacotes e checksums. Para quando você quiser escolher.

O Concat está em **beta**: funciona, e ainda tem arestas. [Conte](https://github.com/jub0t/Concat/issues) quando encontrar uma.

**Plataformas**

- ✅ **Windows** · x86_64 e ARM. Um instalador e um `.msi`. Se o SmartScreen barrar uma build sem assinatura: **Mais informações** › **Executar assim mesmo**
- ✅ **macOS** · Intel e Apple Silicon. Se o macOS se recusar a abrir uma build sem assinatura: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 e ARM. `.deb`, `.rpm`, `.AppImage` e um pacote para Arch. Também um Flatpak no **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, um remoto Flatpak da comunidade que empacota o `.deb` x86_64 de cada versão e se atualiza com ela
- ✅ **Android** · celulares e tablets
- ✅ **iOS / iPadOS** · iPhone e iPad, por sideload

✅ Suportado · 🚧 Em andamento · 🧪 A testar

Os **requisitos do sistema** e os tamanhos dos modelos opcionais estão no [site](https://concatenate.pages.dev/guides/system-requirements).

## Primeiros passos

Baixe, abra, solte o material, corte. Sem conta, sem configuração.

**Para relatar algo:** cada execução grava um log, e em Configurações › Sobre está o botão que o abre, ao lado do que copia as informações do seu sistema. Anexe os dois a uma [issue](https://github.com/jub0t/Concat/issues) e o relato chega com tudo o que precisa. As últimas dez execuções são guardadas, então a de ontem ainda está lá; nada é enviado a lugar nenhum por conta própria.

## Como contribuir

> [!IMPORTANT]
> A melhor forma de contribuir é pegar uma build na página de [Releases](https://github.com/jub0t/Concat/releases) e usá-la: descubra onde ela quebra e diga onde poderia ser melhor.
>
> Pronto para escrever código? O [CONTRIBUTING.md](../CONTRIBUTING.md) cobre a configuração, a organização da árvore, as verificações a rodar e como as contribuições são licenciadas. Controla o Concat a partir de um script, um serviço ou um agente? [A documentação para desenvolvedores](https://concatenate.pages.dev/docs) cobre a API do Concat e seus transportes: JSON-RPC, gRPC e MCP. [Esta Discussion](https://github.com/jub0t/Concat/discussions/3) é onde o projeto foi anunciado.
> 
> Contribuidores podem pedir o papel `@Contributor` no servidor do Discord, basta pedir.

## Contribuidores

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## Histórico de estrelas

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
 </picture>
</a>

<a id="sponsoring"></a>
## Patrocínio

O Concat não tem paywall e nunca terá: sem marca d’água, sem conta, sem plano pago. O patrocínio é o que fica no lugar disso. Se o Concat substituiu uma assinatura para você, uma fração dela o mantém funcionando.

**Para onde vai**

O [roadmap](https://concatenate.pages.dev/roadmap) mostra o que o patrocínio paga, e quanto custa cada parte.

Escolha um nível no [site](https://concatenate.pages.dev/#sponsor).

<a href="https://concatenate.pages.dev/#sponsor"><img src="https://img.shields.io/badge/Sponsor-Concat-0568FD?style=flat-square&labelColor=212123" alt="Sponsor Concat on the website" /></a>

Não pode contribuir agora? Uma estrela, um relato de bug ou uma palavra para alguém que edita vídeo também valem muito.
