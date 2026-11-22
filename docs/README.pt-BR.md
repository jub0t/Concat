<p align="center">
  <img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/concat-mark-dark.png" alt="Concat" width="100" />
</p>

<h1 align="center">Concat</h1>
<h3 align="center">O editor de vídeo gratuito, de código aberto e multiplataforma.</h3>

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

[🇬🇧 English](../README.md) · [🇩🇪 Deutsch](README.de.md) · [🇪🇸 Español](README.es.md) · [🇫🇷 Français](README.fr.md) · [🇮🇹 Italiano](README.it.md) · [🇳🇱 Nederlands](README.nl.md) · [🇵🇱 Polski](README.pl.md) · **🇧🇷 Português (Brasil)** · [🇷🇺 Русский](README.ru.md) · [🇺🇦 Українська](README.uk.md) · [🇹🇷 Türkçe](README.tr.md) · [🇦🇿 Azərbaycanca](README.az.md) · [🇭🇷 Hrvatski](README.hr.md) · [🇭🇺 Magyar](README.hu.md) · [🇮🇩 Bahasa Indonesia](README.id.md) · [🇻🇳 Tiếng Việt](README.vi.md) · [🇹🇭 ไทย](README.th.md) · [🇯🇵 日本語](README.ja.md) · [🇰🇷 한국어](README.ko.md) · [🇨🇳 简体中文](README.zh-Hans.md) · [🇹🇼 繁體中文](README.zh-TW.md) · [🇸🇦 العربية](README.ar.md) · [🇮🇱 עברית](README.he.md) · [🇮🇷 فارسی](README.fa.md) · [🇮🇳 हिन्दी](README.hi.md) · [🇵🇰 اردو](README.ur.md) · [🇧🇩 বাংলা](README.bn.md)

> Traduzido do README em inglês em 11 de outubro de 2026. A versão em inglês é a que se mantém atualizada.

## Patrocinadores pagos

<table width="100%">
  <tr>
    <td align="center">
      <a href="https://proxyon.io/?ref=concat"><img src="https://cdn.jsdelivr.net/gh/jub0t/Concat@main/assets/proxyon_sponsor_logo.png" alt="Proxyon" width="80" /></a><br />
      <a href="https://proxyon.io/?ref=concat"><b>Proxyon</b></a><br />
      <sub>Proxies pagos por uso para desenvolvedores<br />Use o código <code>JUB0T</code> para 20 % de desconto</sub>
    </td>
  </tr>
</table>

</div>


## Sobre

O Concat é um editor de vídeo gratuito e de código aberto para macOS, Windows, Linux, Android e iOS. Ele cobre o que a maioria das edições precisa: legendas automáticas, isolamento de voz, texto para fala, remoção de fundo, animações e keyframes, efeitos e títulos, corte multipista, exportação em 4K. Sem nenhuma pegadinha: sem marca d’água, sem conta, sem assinatura, sem upload.

Tudo roda localmente em um motor nativo em Rust com um compositor na GPU. Instale, solte o material, corte. Os modelos de IA para legendas, vozes e recorte são baixados uma vez nas Configurações e depois funcionam offline. Seu material nunca sai do seu disco.

**Bom para:** TikTok, Reels e Shorts, vídeos do YouTube, tutoriais e gravações de tela, trechos de podcast, memes.

**Também para máquinas:** uma API JSON-RPC, para que scripts e agentes de IA também possam cortar vídeo com ele.

## Destaques

- 🚫 **Sem marca d’água. Sem conta. Sem paywall.** Nunca.
- 🔒 **100% local.** Nada é enviado, nem a IA. Funciona offline.
- 💬 **Legendas automáticas.** No idioma que você fala, geradas no seu computador, estilizadas e já na linha do tempo.
- 🎤 **Isolamento de voz.** Tire uma voz da música ou do ruído, ou remova-a, cada parte no seu próprio nível.
- 🗣️ **Texto para fala + clonagem de voz.** Uma voz local gratuita, ou qualquer voz a partir de alguns segundos de gravação. No desktop.
- 🧍 **Remoção de fundo, chroma key e máscaras.** Pessoas e objetos recortados para você, uma tela verde que se recorta sozinha e uma pilha de máscaras em cada clipe, cada uma com seu próprio modo e seus keyframes.
- 🌀 **Animações de entrada, saída e loop.** Para clipes, imagens, títulos e formas. Cada uma toca no seu cartão antes de você escolher.
- ✨ **Mais de 100 efeitos, filtros e transições, 25 efeitos sonoros.** Visuais cinematográficos, tremores e impactos, retrô, cortinas, tudo ao vivo na prévia. Uma transição é um clipe que você move e apara como qualquer outro.
- 🎞️ **Keyframes.** Posição, escala, tamanho, rotação, opacidade, volume, parâmetros de efeitos. Editor de curvas embutido.
- 🏃 **Rampas e curvas de velocidade.** Constantes ou desenhadas, suaves ou retas. Congelamento de quadro e reverso.
- 🪄 **Melhorar e Estabilizar.** Recupere imagens suaves ou antigas, até 2K. Acalme uma cena tremida.
- ✂️ **Corte rápido, corte exato.** Dividir, aparar, ripple, mesclar, legendas e sobreposições vinculadas, linha do tempo magnética se você quiser. Todo corte cai no quadro, igual na prévia e na exportação.
- 🎚️ **Multipista, multilinha do tempo.** Empilhe vídeo, imagens, títulos e formas em quantas pistas quiser. Vários cortes em um projeto. Modos de mesclagem, recorte, espelhamento.
- 📝 **Títulos e formas.** Fontes, contorno, sombra, placas, emoji coloridos e predefinições para árabe, chinês, hindi, tailandês e mais oito idiomas. Formas com gradientes, estrelas, contornos e sombras.
- 🎙️ **Narração e limpeza de voz com um interruptor.** Grave sobre o seu corte. Remoção de ruído, melhoria da voz, nivelamento do volume. Mais esquilo, robô, telefone e companhia.
- 📤 **Exportação.** H.264, HEVC, AV1, com o codificador do próprio computador. Até 4K 60, cor de 10 bits.
- ⚡ **Rápido.** Proxies embutidos, para o 4K rolar com fluidez. Os títulos são desenhados em todos os núcleos, e cada efeito fica pronto assim que a janela abre.
- 🛟 **Continua em qualquer máquina.** Quando o chip gráfico desiste, a prévia continua desenhando.
- 🤖 **Automatizável.** Uma API JSON-RPC com a qual scripts e agentes de IA podem cortar vídeo. Compilando a partir do código-fonte, você ganha também gRPC e uma ferramenta de linha de comando.
- 🖥️ **macOS, Windows, Linux, Android, iOS.** 27 idiomas. O mesmo app, os mesmos arquivos de projeto.

## Download

Dois caminhos:

1. **[O site](https://concat.standalones.org/#download)** entrega a build certa para a sua máquina. Comece por aqui.
2. **[GitHub Releases](https://github.com/jub0t/Concat/releases)** tem todas as builds para todas as plataformas, com instaladores, pacotes e checksums. Para quando você quiser escolher.

O Concat está em **beta**: funciona, e ainda tem arestas. [Conte](https://github.com/jub0t/Concat/issues) quando encontrar uma.

**Plataformas**

- ✅ **Windows** · x86_64 (as builds para ARM estão pausadas, veja [#326](https://github.com/jub0t/Concat/issues/326)). Um instalador e um `.msi`. Se o SmartScreen barrar uma build sem assinatura: **Mais informações** › **Executar assim mesmo**
- ✅ **macOS** · Intel e Apple Silicon. Se o macOS se recusar a abrir uma build sem assinatura: `xattr -dr com.apple.quarantine /Applications/Concat.app`
- ✅ **Linux** · x86_64 e ARM. `.deb`, `.rpm`, `.AppImage` e um pacote para Arch. Também um Flatpak no **[Flatpark](https://flatpark.org/apps/app.concat.editor/)**, um remoto Flatpak da comunidade que empacota o `.deb` x86_64 de cada versão e se atualiza com ela
- ✅ **Android** · celulares e tablets
- ✅ **iOS / iPadOS** · iPhone e iPad, por sideload

✅ Suportado · 🚧 Em andamento · 🧪 A testar

Os **requisitos do sistema** e os tamanhos dos modelos opcionais estão no [site](https://concat.standalones.org/guides/system-requirements).

**[Code signing policy](../CODE_SIGNING.md)**

## Como contribuir

> [!IMPORTANT]
> A melhor forma de contribuir é pegar uma build na página de [Releases](https://github.com/jub0t/Concat/releases) e usá-la: descubra onde ela quebra e diga onde poderia ser melhor.
>
> Pronto para escrever código? O [CONTRIBUTING.md](../CONTRIBUTING.md) cobre a configuração, a organização da árvore, as verificações a rodar e como as contribuições são licenciadas. Controla o Concat a partir de um script, um serviço ou um agente? [A documentação para desenvolvedores](https://concat.standalones.org/docs) cobre a API do Concat e seus transportes: JSON-RPC e gRPC. [Esta Discussion](https://github.com/jub0t/Concat/discussions/3) é onde o projeto foi anunciado.
> 
> Contribuidores podem ganhar o cargo `@Contributor` no servidor do Discord: basta pedir.

## Histórico de estrelas

<a href="https://www.star-history.com/?repos=jub0t%2Fconcat&type=date&releases=&legend=bottom-right">
 <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jub0t/concat&type=date&legend=bottom-right" />
</a>

## Contribuidores

<a href="https://github.com/jub0t/Concat/graphs/contributors">
  <img alt="Contributors" src="https://contrib.rocks/image?repo=jub0t/concat">
</a>

## DMCA

Se você acredita que algo neste repositório, em suas versões publicadas ou em um pacote ou modelo que ele distribui viola seus direitos autorais, envie um e-mail para [jub0trd@gmail.com](mailto:jub0trd@gmail.com) com uma descrição da obra de sua titularidade, um link para o local onde o material infrator aparece aqui, seus dados de contato, uma declaração de que você acredita de boa-fé que o uso não está autorizado por você, por seu representante ou pela lei, e uma declaração, sob pena de perjúrio, de que a notificação é exata e de que você é o titular ou está autorizado a agir em nome dele, com sua assinatura física ou eletrônica. Notificações válidas são atendidas prontamente, e quem tiver material removido pode enviar uma contranotificação para o mesmo endereço.
