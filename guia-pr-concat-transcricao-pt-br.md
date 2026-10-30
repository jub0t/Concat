# Guia de contribuição: transcrição automática em português brasileiro no Concat

## 1. Objetivo do PR

Permitir que usuários do aplicativo Concat gerem transcrições/legendas automáticas de áudio em português brasileiro, aproveitando a infraestrutura existente de Whisper.

**Ponto importante:** o Concat já anuncia legendas automáticas com Whisper local. Portanto, antes de implementar algo novo, confirmar se português já funciona e identificar a lacuna real. O PR pode ser uma correção de idioma, exposição de uma opção na interface, melhoria da seleção de modelos ou documentação/testes — não necessariamente um novo sistema de transcrição.

Este guia não pressupõe que exista um bug confirmado. Nenhum código do motor foi compilado ou testado nesta análise; os caminhos de arquivos específicos além dos documentados devem ser localizados no checkout.

## 2. Primeiro passo: alinhar com os mantenedores

O CONTRIBUTING.md solicita abrir uma issue ou conversar no Discord antes de alterações maiores que uma pequena correção.

Antes de abrir a issue:

1. Procurar issues e PRs sobre Portuguese, Brazilian Portuguese, Whisper, transcription, captions e language selection.
2. Instalar uma versão recente do aplicativo.
3. Baixar um modelo Whisper multilíngue nas configurações.
4. Testar uma gravação curta em português brasileiro.
5. Registrar versão, sistema operacional, modelo, seleção de idioma disponível e resultado.
6. Conferir se a falha também ocorre na revisão de desenvolvimento que será usada no PR.

Perguntas a responder:

- A transcrição já reconhece português automaticamente?
- É possível selecionar português explicitamente?
- O aplicativo usa por engano o idioma da interface como idioma da fala?
- O código de locale `pt-BR` chega ao backend sem conversão para `pt`?
- Há modelos apenas em inglês sendo oferecidos para português?
- O problema está no reconhecimento, na criação das legendas ou apenas na documentação?

### Modelo de issue em inglês

Título:

```text
Improve Brazilian Portuguese support in automatic captions
```

Corpo sugerido — preencher sem afirmar resultados que não foram observados:

```markdown
## Goal
Make automatic captions reliable and discoverable for Brazilian Portuguese
using the existing local Whisper integration.

## Current behavior
- Concat version/commit:
- OS:
- Whisper model:
- Language setting:
- Steps to reproduce:
- Actual result:

## Expected behavior
Users can transcribe Brazilian Portuguese speech into Portuguese captions.
If manual language selection is available, Portuguese maps to Whisper's `pt`
language code, and transcription does not translate the output into English.

## Proposed scope
After checking existing support:
- Reuse concat-speech and the current captions workflow.
- Preserve automatic language detection.
- Add or fix explicit Portuguese selection if needed.
- Validate multilingual model compatibility.
- Add focused tests and user-facing documentation.

Is this already being worked on? Would you prefer Portuguese as one language
option or an explicit Brazilian Portuguese label mapped to `pt`?
```

## 3. Preparar o ambiente

Requisitos documentados pelo projeto:

- Git.
- Rust: consultar `rust-version` em `src/Cargo.toml`; o README consultado informa 1.93 ou superior.
- Bibliotecas de desenvolvimento do FFmpeg 7 ou superior.
- CMake e compilador C++.
- Dependências gráficas específicas da plataforma; no Linux, consultar o workflow de build para a lista, incluindo fontconfig/freetype.

No Windows, o README recomenda o build BtbN `win64-gpl-shared`, com `FFMPEG_DIR` apontando para o diretório descompactado. Ter somente `ffmpeg.exe` não substitui os headers e bibliotecas de desenvolvimento necessários para compilar.

### Fork e branch

Criar um fork no GitHub e executar, substituindo SEU_USUARIO:

```bash
git clone https://github.com/SEU_USUARIO/Concat.git
cd Concat
git remote add upstream https://github.com/jub0t/Concat.git
git switch -c fix/portuguese-auto-captions
```

Ler os documentos atuais antes de alterar código:

- `CONTRIBUTING.md`
- `src/README.md`
- `CLA.md`
- `TRANSLATING.md`

Executar o aplicativo:

```bash
cd src
cargo run -p concat
```

Executar primeiro sem alterações para separar problemas do ambiente de regressões do PR.

## 4. Onde investigar

A arquitetura documentada identifica:

| Local | Papel |
| --- | --- |
| `src/crates/concat-speech/` | Transcrição com whisper.cpp via whisper-rs; também síntese de voz |
| `src/crates/concat-host/` | Sessões, mídia, jobs e infraestrutura usada pela janela |
| `src/crates/concat/` | Interface Slint, diálogos e ações do usuário |
| `models/manifest.toml` | Manifesto de modelos baixáveis |
| `TRANSLATING.md` | Processo de tradução da interface |
| `scripts/locales.py` | Atualização do inventário de strings |

Não assumir que um arquivo ou função tenha determinado nome. Localizar o fluxo real com busca no repositório.

Exemplos com ripgrep, a partir da raiz:

```bash
rg -n "whisper|transcrib|caption" src/crates/concat-speech src/crates/concat src/crates/concat-host
rg -n "set_language|set_translate|detect_language|language" src/crates/concat-speech
rg -n "pt-BR|pt_BR|Portuguese|portugu" src
rg -n "whisper|multilingual|\.en" models src/crates/concat-speech
```

Se ripgrep não estiver instalado, usar a busca global do editor.

Traçar o caminho completo:

```text
Ação de gerar legendas
    -> opções de transcrição
    -> modelo selecionado
    -> preparação do áudio
    -> parâmetros enviados ao Whisper
    -> segmentos reconhecidos
    -> conversão para legendas na timeline
    -> atualização da interface e persistência do projeto
```

Registrar no PR os arquivos e funções realmente encontrados. Verificar também se já existem testes e como são organizados.

## 5. Decisões de implementação

### 5.1 Reutilizar a integração existente

Usar `concat-speech`, whisper-rs e os padrões atuais de jobs/progresso/cancelamento. Não adicionar um servidor web, serviço externo, Python ou faster-whisper ao aplicativo apenas para habilitar português.

Um idioma adicional normalmente deve ser tratado como configuração da infraestrutura existente, não como um segundo motor.

### 5.2 Locale da interface não é idioma do modelo

Whisper usa `pt` para português. `pt-BR` é um identificador de locale, não um modelo separado de reconhecimento de português brasileiro.

Se a aplicação aceitar locales, normalizar na fronteira com o backend, seguindo os padrões existentes. Exemplo de comportamento desejado:

| Opção/configuração | Valor semântico para Whisper |
| --- | --- |
| Detecção automática | Usar o mecanismo de autodetecção da versão instalada |
| Português | `pt` |
| Português (Brasil), se aprovado pelos mantenedores | `pt` |
| `pt-BR`/`pt_BR`, se aceitos na configuração | `pt` |

Não usar uma regra genérica de cortar qualquer código no hífen sem verificar a lista de idiomas e o comportamento existente. Não inventar códigos para idiomas desconhecidos: preservar a política atual de erro ou validação.

A forma exata de representar autodetecção deve ser conferida na versão de whisper-rs utilizada; não presumir que seja uma string ou `None` específicos.

### 5.3 Preservar o idioma falado

A tarefa deve ser transcrição, não tradução para inglês. Verificar os parâmetros usados na integração Whisper e garantir que a opção de tradução esteja desativada no fluxo normal de legendas.

Não forçar português para todos os usuários. Preservar autodetecção e outros idiomas.

Se o usuário escolher explicitamente português, essa opção deve prevalecer sobre o locale da interface. Uma interface em inglês deve conseguir transcrever áudio em português, e vice-versa.

### 5.4 Modelos multilíngues

Utilizar modelos Whisper multilíngues. Modelos com sufixo `.en` são voltados ao inglês.

Verificar se o aplicativo já distingue os modelos. Se necessário, seguir o padrão existente para avisar ou impedir uma combinação incompatível entre idioma e modelo, sem substituir silenciosamente um download grande.

Não é necessário adicionar um novo modelo somente porque o áudio é brasileiro. Caso realmente seja adicionado/alterado um modelo baixável:

1. Atualizar `models/manifest.toml`.
2. Atualizar a tabela correspondente do motor.
3. Rodar `python3 scripts/models.py --check` na raiz.
4. Seguir o fluxo dos mantenedores para espelhamento e digests.

### 5.5 Interface e persistência

Se a seleção de idioma já existir, corrigir sua integração antes de criar outra.

Se for necessário adicionar um seletor, discutir o escopo com os mantenedores. Um seletor geral com Auto e os idiomas já suportados pode fazer mais sentido que uma opção especial isolada para pt-BR.

Se a opção for persistida:

- Definir um padrão compatível com projetos/configurações antigas.
- Não quebrar a leitura de arquivos existentes.
- Usar o mecanismo de configurações e serialização do projeto.

Não prometer ortografia exclusivamente brasileira apenas por selecionar `pt`: a saída precisa ser avaliada e pode requerer revisão manual.

### 5.6 Legendas e tempos

Preservar a conversão temporal existente. Não alterar o sistema de timestamps para corrigir seleção de idioma.

Testar a sincronização com clips aparados, deslocados na timeline ou com velocidade alterada, conforme o fluxo atual suportar essas operações. Determinar se o áudio transcrito é o arquivo original, uma seleção ou o áudio consolidado da timeline.

## 6. Estratégia de testes

### 6.1 Testes unitários rápidos

Priorizar testes que não baixem modelos nem executem reconhecimento pesado:

- Português produz o código `pt` na construção dos parâmetros.
- `pt-BR`/`pt_BR` são normalizados se esses formatos fizerem parte da entrada aceita.
- Autodetecção continua funcionando como configuração padrão.
- Outros idiomas mantêm o comportamento anterior.
- Idiomas inválidos seguem a política de validação existente.
- Transcrição não ativa tradução.
- Configurações antigas continuam carregando, se houver mudança de schema.
- Modelos apenas em inglês são tratados de forma coerente quando o idioma escolhido é português.

Testar no nível em que o projeto permite observar os parâmetros; evitar refatorações extensas apenas para um teste.

### 6.2 Teste de integração com áudio

Preferir uma gravação curta feita por você, com autorização clara para sua inclusão. Não usar material de terceiros sem licença ou gravações contendo dados pessoais.

Texto sugerido:

> Olá! Esta é uma gravação de teste em português brasileiro. Hoje vamos editar um vídeo, adicionar legendas e conferir a sincronização do áudio.

Resultados a verificar:

- O resultado permanece em português.
- Acentos e caracteres Unicode são preservados.
- As legendas aparecem na posição esperada.
- Tempos são válidos e ordenados.
- Cancelamento e progresso continuam funcionando.

Evitar exigir correspondência literal perfeita de toda a transcrição: pontuação e segmentação podem variar entre modelos, versões e hardware. Utilizar asserções robustas e revisão manual para qualidade linguística.

Se o teste precisar de modelo grande, seguir o padrão do repositório para testes opcionais/ignorados e documentar como executá-lo. Não colocar download pesado na suíte padrão sem aprovação.

### 6.3 Matriz manual

| Caso | Resultado esperado |
| --- | --- |
| Fala brasileira limpa, idioma Auto | Português reconhecido, dentro da qualidade do modelo |
| Mesma fala, português explícito | Transcrição em português |
| Interface em inglês + fala brasileira | Português continua disponível |
| Interface em português + fala em inglês | Não forçar português por causa da interface |
| Áudio com silêncio | Não travar; revisar possíveis alucinações do modelo |
| Modelo incompatível com português | Aviso/validação conforme o comportamento projetado |
| Cancelar transcrição | Job finaliza sem aplicar resultado parcial indevido |
| Exportar projeto legendado | Texto legível e sincronizado |
| Salvar e reabrir | Legendas e configurações persistem conforme esperado |

Registrar modelo, hardware, duração do áudio e tempo de execução. Não afirmar ganho de precisão sem comparar amostras nas mesmas condições.

## 7. Strings, logs e estilo

- Reutilizar o sistema de internacionalização; não inserir textos fixos apenas em português.
- Consultar `TRANSLATING.md` antes de editar traduções.
- Ao adicionar/alterar texto visível, executar da raiz:

```bash
python3 scripts/locales.py
```

No Windows, `python` pode ser o comando disponível em vez de `python3`.

- Usar a fachada `log`, como `log::info!`, `log::warn!` e `log::error!`, em vez de prints.
- Não registrar o áudio ou o texto completo do usuário desnecessariamente.
- Para diagnóstico local, o projeto documenta `CONCAT_LOG=debug`.
- Manter nomes, organização e estilo próximos do código existente.

Novos arquivos-fonte precisam do cabeçalho documentado, adaptando o marcador de comentário ao tipo de arquivo:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
```

## 8. Executar verificações

No diretório `src`:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Durante o desenvolvimento, executar os testes focados no crate alterado pode economizar tempo, por exemplo:

```bash
cargo test -p concat-speech
```

Esse comando depende das dependências nativas e da configuração atual do crate. Testes focados não substituem as verificações completas solicitadas pelo projeto.

Se a formatação precisar de correção, rodar `cargo fmt`, revisar o diff e executar novamente os checks. Não misturar mudanças de formatação não relacionadas ao PR.

## 9. Preparar o commit e o PR

Revisar:

```bash
git status
git diff --check
git diff
```

Adicionar somente os arquivos necessários com `git add <caminhos>`.

**Ler `CLA.md` antes de assinar:** o CONTRIBUTING consultado informa que contribuições de código são aceitas sob um acordo de cessão dos direitos autorais ao proprietário do Concat, com licença de retorno e compromisso descritos no CLA. Não confundir o sign-off com uma simples formalidade ou com um DCO genérico.

Se concordar com o acordo:

```bash
git commit -s -m "fix: improve Portuguese automatic captions"
git push -u origin fix/portuguese-auto-captions
```

Abrir PR do fork para a branch definida pelo projeto, normalmente main. Correções exclusivamente documentais ou de digitação têm exceção de sign-off segundo o CONTRIBUTING consultado; conferir a versão atual.

### Modelo de descrição de PR

```markdown
## Summary
- Describe the verified gap in Portuguese automatic captions.
- Explain the smallest change made to the existing Whisper integration.
- Link the issue/discussion and state what is intentionally out of scope.

## Implementation
- Language selection/normalization:
- Whisper parameters:
- Model compatibility:
- UI/localization changes:
- Backward compatibility:

## Testing
- [ ] cargo fmt --check
- [ ] cargo clippy --all-targets -- -D warnings
- [ ] cargo test
- [ ] Locale inventory updated if UI text changed
- [ ] Brazilian Portuguese audio tested manually
- [ ] Automatic detection and another language checked
- [ ] Captions reviewed in preview and exported video

Test environment:
- OS:
- Commit:
- Model:
- Hardware:
- Sample duration:

## Evidence
Attach screenshots or a short recording if useful.
Only include audio/video you are authorized to share.

## Limitations
Document untested platforms, model variability, and remaining issues.
```

Marcar apenas verificações realmente executadas. Se alguma falhar por ambiente, informar comando, erro e impacto, em vez de indicar aprovação.

## 10. Checklist final

- [ ] Confirmei que há uma lacuna real; não dupliquei suporte já existente.
- [ ] Procurei issues/PRs e alinhei alterações maiores com mantenedores.
- [ ] Reutilizei o Whisper existente, sem criar outro pipeline desnecessário.
- [ ] Usei `pt` na fronteira com o motor quando português foi escolhido.
- [ ] Preservei Auto e os outros idiomas.
- [ ] Não confundi idioma da interface, idioma do áudio e tradução.
- [ ] Considerei modelos `.en` versus multilíngues.
- [ ] Revisei timestamps e exportação de legendas.
- [ ] Adicionei testes proporcionais à mudança.
- [ ] Atualizei strings/documentação quando necessário.
- [ ] Não incluí modelos grandes, dados pessoais ou áudio sem permissão.
- [ ] Li o CLA e fiz sign-off se aplicável e aceito.
- [ ] Descrevi honestamente o que foi e não foi testado.

## 11. Escopo recomendado para o primeiro PR

O melhor primeiro PR é a menor correção comprovada pelo teste inicial:

1. **Português já funciona:** melhorar documentação, descoberta na interface ou adicionar cobertura de teste.
2. **Auto funciona, seleção explícita não:** corrigir a configuração ou adicionar a opção, após alinhamento.
3. **Locale chega incorretamente ao Whisper:** normalizar a entrada com testes.
4. **Modelo inglês é utilizado indevidamente:** corrigir seleção/validação de compatibilidade.
5. **Reconhecimento funciona, legendas falham:** corrigir a conversão temporal ou criação das legendas, sem alterar o motor linguístico sem necessidade.

Evitar um PR que combine redesign da UI, atualização de Whisper, modelos novos, refatoração geral e pt-BR ao mesmo tempo.

## 12. Fontes e limites

Documentos do Concat consultados via webfetch nesta conversa:

- https://github.com/jub0t/Concat
- https://github.com/jub0t/Concat/blob/main/CONTRIBUTING.md
- https://raw.githubusercontent.com/jub0t/Concat/main/src/README.md
- https://raw.githubusercontent.com/jub0t/Concat/main/docs/MCP.md

Ler no checkout antes de contribuir:

- https://github.com/jub0t/Concat/blob/main/CLA.md
- https://github.com/jub0t/Concat/blob/main/TRANSLATING.md
- https://github.com/jub0t/Concat/blob/main/models/manifest.toml

Este arquivo é um roteiro para investigação e contribuição, não a implementação do recurso nem a confirmação de que pt-BR esteja ausente. A branch main pode mudar; sempre validar contra a revisão escolhida para o PR.
