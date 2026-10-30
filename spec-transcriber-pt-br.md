# SDD — Transcrição explícita em português brasileiro

**Status:** proposta para análise; implementação não autorizada por este documento.  
**Data:** 2026-10-29.  
**Referência:** `guia-pr-concat-transcricao-pt-br.md`.  
**Escopo desta entrega:** investigação estática e especificação, sem alterações no aplicativo.

## 1. Parecer de viabilidade

**É viável oferecer transcrição explícita de fala brasileira reutilizando o `Transcriber` existente. Não é necessário criar um `TranscriberPtBr` nem duplicar `transcribe.rs`.**

O transcritor atual não é exclusivo do inglês. Ele oferece modelos ingleses e multilíngues; os multilíngues usam autodetecção. Portanto, português já possui um caminho técnico disponível, mas seu funcionamento e qualidade neste ambiente ainda precisam de ensaio com áudio real.

A lacuna confirmada por leitura do código é a ausência de seleção explícita do idioma. A proposta torna português selecionável, mantém o fluxo inglês e não confunde idioma da interface com idioma do áudio.

Whisper identifica português como `pt`, não `pt-BR`. O rótulo “Português (Brasil)” descreve a intenção do usuário, mas não garante vocabulário ou ortografia exclusivamente brasileiros. Não há justificativa para inventar modelos `base.pt` ou `base.pt-BR`.

## 2. Evidências do estado atual

As referências abaixo correspondem ao checkout analisado; números de linha podem mudar na implementação.

| Evidência | Local |
| --- | --- |
| Modelos `tiny.en`, `base.en`, `small.en` e equivalentes multilíngues `tiny`, `base`, `small` | `src/crates/concat-speech/src/transcribe.rs:55` |
| Request sem campo de idioma | `src/crates/concat-speech/src/transcribe.rs:343` |
| Exclusão mútua, validação e áudio mono float32 a 16 kHz | `src/crates/concat-speech/src/transcribe.rs:503` |
| Contexto multilíngue recebe `auto`; contexto inglês recebe `en` | `src/crates/concat-speech/src/transcribe.rs:569` |
| VAD, inferência, cancelamento e extração dos segmentos | `src/crates/concat-speech/src/transcribe.rs:615` |
| Teste manual ignorado com arquivo e modelo configuráveis | `src/crates/concat-speech/src/transcribe.rs:671` |
| Teste real ignorado usa `tiny.en` | `src/crates/concat-speech/src/transcribe.rs:760` |
| Diálogo oferece áudio e modelo, sem idioma | `src/crates/concat/ui/dialogs/captions.slint:185` |
| Request é construído a partir da janela do clipe | `src/crates/concat/src/panes/captions.rs:471` |
| Conversão temporal e aplicação em lote com undo | `src/crates/concat/src/panes/captions.rs:190` |
| Preferências de modelo e locale da interface são distintas | `src/crates/concat/src/prefs.rs:47` |
| Seleção de modelo nas configurações é persistida | `src/crates/concat/src/panes/settings.rs:431` |
| Dependência whisper-rs 0.16; resolução 0.16.0 | `src/crates/concat-speech/Cargo.toml:46`; `src/Cargo.lock:8033` |
| Código `pt` presente no Whisper vendorizado | `src/patches/whisper-rs-sys/whisper.cpp/src/whisper.cpp:289` |
| Default nativo de tradução é `false` | `src/patches/whisper-rs-sys/whisper.cpp/src/whisper.cpp:5906` |

Não há chamada explícita a `set_translate(false)` no fluxo atual, mas o default nativo já desabilita tradução. Explicitá-la será uma proteção do contrato, não correção de uma tradução indevida comprovada.

A investigação encontrou um caller de produção em `panes/captions.rs` e dois literais do request nos testes de `transcribe.rs`. Não foi encontrado endpoint de transcrição no MCP: `captions_add` recebe legendas prontas (`src/crates/concat-mcp/src/tools.rs:775`).

## 3. Objetivo e limites

### Objetivo

Permitir selecionar português brasileiro ao gerar legendas de um clipe, usando modelos multilíngues locais existentes e preservando progresso, cancelamento, timestamps, preview, exportação e undo.

### Fora do escopo

- Novo motor, servidor, serviço de nuvem ou dependência Python no aplicativo.
- Treinamento/fine-tuning, download automático ou novos modelos.
- Atualização do Whisper, mudanças no manifesto de modelos ou algoritmos de VAD.
- Tradução, correção ortográfica brasileira automática ou identificação regional.
- Redesign das configurações, novo endpoint MCP/API ou alteração do schema do projeto.
- Transcrição da mixagem inteira da timeline ou detecção por segmento em áudio multilíngue.
- Promessas de precisão ou velocidade equivalentes ao inglês sem medição.

## 4. Requisitos normativos

| ID | Requisito |
| --- | --- |
| RF-01 | O diálogo de legendas DEVE permitir escolher Automático ou Português (Brasil). |
| RF-02 | Automático DEVE ser o padrão e preservar `auto` para modelos multilíngues e `en` para modelos ingleses. |
| RF-03 | Português explícito DEVE enviar `pt` ao Whisper multilíngue. |
| RF-04 | Português explícito com modelo somente inglês DEVE falhar com orientação para selecionar um modelo multilíngue, sem inferência ou resultado parcial. |
| RF-05 | A aplicação NÃO DEVE trocar ou baixar modelo silenciosamente. |
| RF-06 | A tarefa DEVE manter tradução desativada explicitamente. |
| RF-07 | O idioma da interface NÃO DEVE determinar o idioma da transcrição. |
| RF-08 | Requests antigos sem idioma DEVEM manter o comportamento atual. |
| RF-09 | A escolha DEVE permanecer estável durante uma execução; modificações não podem afetar um job em andamento. |
| RF-10 | Novas strings visíveis DEVEM seguir a internacionalização existente. |
| RNF-01 | Processamento DEVE permanecer local, sem registrar áudio ou texto integral desnecessariamente. |
| RNF-02 | Testes rápidos NÃO DEVEM baixar modelos ou executar reconhecimento pesado. |
| RNF-03 | Decodificação, cache, controle de concorrência, VAD e cálculo temporal DEVEM ser reutilizados. |

## 5. Contrato proposto

### 5.1 Representação do idioma

Adicionar a `TranscribeRequest` um campo `language: Option<TranscriptionLanguage>` com default de desserialização. Definir um enum com representações serializadas `auto` e `pt`. Os nomes Rust devem seguir as convenções do crate.

| Entrada serializada | Significado |
| --- | --- |
| Campo ausente ou `null` | Automático, compatível com requests antigos |
| `"auto"` | Automático explícito |
| `"pt"` | Português explícito |
| `"pt-BR"`, `"pt_BR"` ou outro valor | Erro de desserialização no contrato inicial |

O escopo mínimo não aceita locales como entrada. A UI mapeia seu rótulo diretamente para `pt`; não precisa cortar códigos no hífen ou normalizar strings arbitrárias. Caso aliases regionais sejam aprovados posteriormente, devem ser adicionados explicitamente e cobertos por testes.

Os demais idiomas continuam disponíveis por autodetecção, como hoje. A seleção explícita de todos os idiomas não faz parte desta primeira entrega.

### 5.2 Resolução e validação

| Opção efetiva | Modelo multilíngue | Modelo inglês |
| --- | --- | --- |
| Automático | `auto` | `en` |
| Português | `pt` | Erro orientativo |

Extrair apenas a lógica mínima necessária para resolver idioma/compatibilidade em função testável, sem uma refatoração geral do pipeline.

1. Validar a escolha contra `KnownModel.english_only` antes de decodificar áudio ou carregar modelo incompatível.
2. Após carregar/reutilizar o contexto, verificar também `context.is_multilingual()`, autoridade sobre o arquivo efetivamente carregado.
3. Resolver o código de idioma e construir os parâmetros da execução.
4. Chamar `set_language(...)` e `set_translate(false)`.
5. Manter inalterados os demais parâmetros, callbacks e tratamento de resultados.

Mensagem semântica esperada: “Português requer um modelo multilíngue. Selecione Tiny, Base ou Small (Multilingual) nas configurações do transcritor.” A redação e tradução finais devem seguir os padrões do aplicativo.

O cache continua identificado pelo modelo: mudar apenas o idioma não exige recarregá-lo, pois cada execução cria seu próprio estado e parâmetros.

### 5.3 Compatibilidade

- Compatibilidade de desserialização: campo ausente significa Automático.
- Compatibilidade de comportamento: usuários que não selecionam português mantêm o fluxo atual.
- Compatibilidade Rust: adicionar um campo exige atualizar todos os literais de `TranscribeRequest`; não é uma mudança totalmente compatível na API fonte.
- Schema do projeto: nenhuma alteração; legendas continuam sendo títulos existentes.
- Preferências: **proposta mínima sem persistência entre sessões**. A escolha pertence ao diálogo e começa em Automático ao inicializá-lo. Reutilizar o mecanismo existente de estado do painel sem alterar a preferência global de modelo.

Persistir `transcriberLanguage` em `prefs.rs` fica como extensão separada, condicionada à aprovação. Se incluída depois, ausência deve equivaler a Automático e o campo deve continuar independente de `locale`.

## 6. UX e integração

Adicionar o seletor de idioma junto aos controles de modelo no diálogo existente, com duas opções internacionalizadas:

- Automático — comportamento atual.
- Português (Brasil) — exige modelo multilíngue; usa `pt`.

Quando português e um modelo inglês estiverem selecionados, mostrar orientação e impedir o início pela UI. O backend deve repetir a validação: a UI não é uma fronteira de confiança.

Manter a seleção de modelo visível e não ocultar a incompatibilidade por troca automática. Se nenhum modelo compatível estiver instalado, orientar para Settings > Transcriber; o download continua sendo uma ação explícita do usuário. `base` é candidato inicial para avaliação, não uma recomendação de qualidade comprovada.

A escolha deve ser capturada no início do job. Durante a execução, desabilitar os controles relacionados conforme o padrão atual do diálogo, preservando o cancelamento.

### Fluxo alvo

```text
Clipe + faixa de áudio + modelo instalado + idioma do diálogo
    -> snapshot da requisição e validação de compatibilidade
    -> janela original do clipe em mono float32 / 16 kHz
    -> contexto Whisper existente e validação da capacidade real
    -> idioma auto/en/pt + tradução desativada
    -> segmentos relativos à janela, com VAD existente
    -> transformação para timeline e aplicação em lote com undo
```

O áudio atual é o trecho do arquivo original, não uma mixagem consolidada. Preservar `source_start`, janela `duration * speed` e transformação `clip.start + segment.start / speed`; não alterar esses cálculos para adicionar idioma.

## 7. Arquivos previstos

| Arquivo | Alteração proposta |
| --- | --- |
| `src/crates/concat-speech/src/transcribe.rs` | Tipo de idioma, campo opcional, resolução/validação, tradução explícita, atualização dos literais e testes. |
| `src/crates/concat/src/panes/captions.rs` | Estado do seletor, compatibilidade na UI e captura do idioma no request. |
| `src/crates/concat/ui/dialogs/captions.slint` | Seletor, estado indisponível durante execução e mensagem de incompatibilidade. |
| Arquivos de ligação/exportação de tipos, se necessários | Somente os ajustes exigidos pela integração e compilação. |
| Inventário/arquivos de tradução utilizados pelo projeto | Atualização pelo fluxo de `TRANSLATING.md`; revisar o diff gerado. |

Sem mudanças previstas em `models/manifest.toml`, no download, no MCP ou em `prefs.rs` no escopo mínimo.

## 8. Critérios de aceitação e testes

### 8.1 Testes determinísticos, sem modelo

| ID | Cenário | Resultado exigido |
| --- | --- | --- |
| T-01 | Desserializar request antigo | Idioma efetivo Automático. |
| T-02 | Campo `null` ou `auto` | Mesma política de Automático. |
| T-03 | Português com capacidade multilíngue | Resolução produz `pt`. |
| T-04 | Automático com capacidade multilíngue/inglesa | Produz `auto`/`en`, respectivamente. |
| T-05 | Português com modelo `.en` | Erro antes da leitura do áudio; nenhuma inferência. |
| T-06 | Metadados multilíngues, contexto real inglês | Validação após carregamento impede português. |
| T-07 | `pt-BR`, `pt_BR` ou valor desconhecido | Rejeição, sem fallback silencioso. |
| T-08 | Construção dos parâmetros | Tradução explicitamente desativada; testar pela superfície disponível sem depender de detalhes privados do wrapper. |
| T-09 | Segmento sintético com “Olá, edição e ação” | Unicode preservado pelo processamento de segmentos. |
| T-10 | Troca do idioma do diálogo | Novo request recebe a escolha; locale da interface não interfere. |

T-09 verifica transporte de texto, não precisão do reconhecimento. Para T-08, confirmar na implementação a API disponível para observar parâmetros; se necessário, testar uma política pequena compartilhada pela construção real, sem criar uma camada extensa apenas para testes.

Manter os testes existentes de VAD, descarte de segmentos, fontes de modelos e status. Atualizar os dois literais dos testes existentes com idioma Automático.

### 8.2 Integração opcional com áudio brasileiro

Estender o padrão de testes ignorados existente para receber idioma explícito, preservando seus defaults. Proposta: acrescentar `CONCAT_CAPTION_LANGUAGE` com `auto` ou `pt` ao teste `captions_of_a_file`. Esta variável **ainda não existe**.

Usar gravação própria/autorizada de 15–30 segundos, sem dados pessoais, contendo, por exemplo:

> Olá! Esta é uma gravação de teste em português brasileiro. Hoje vamos editar um vídeo, adicionar legendas e conferir a sincronização do áudio.

Executar com o mesmo áudio e modelo multilíngue em Automático e Português. Verificar resultado não vazio, texto em português, preservação de acentos e tempos finitos, positivos em duração e ordenados. Avaliar manualmente sincronização e conteúdo; não exigir igualdade literal entre modelos/hardware.

O teste deve exigir explicitamente áudio e modelo disponíveis, sem indicar sucesso falso quando faltarem. Não incluir download na suíte padrão nem versionar áudio sem autorização.

### 8.3 Matriz manual obrigatória antes de concluir a implementação

- [ ] Fala brasileira, `base` ou outro modelo multilíngue, Automático.
- [ ] Mesmo áudio/modelo, Português explícito.
- [ ] Interface inglesa e Português explícito.
- [ ] Interface portuguesa, fala inglesa, Automático, com modelo inglês e multilíngue.
- [ ] Português com cada família `.en`: início impedido, orientação clara, nenhum download automático.
- [ ] Ausência de modelo multilíngue instalado: orientação para instalação explícita.
- [ ] Cancelamento, progresso e nova execução após cancelamento.
- [ ] Silêncio/ruído: sem travamento; registrar possíveis alucinações, sem prometer ausência delas.
- [ ] Clipe aparado, deslocado e com velocidade alterada: preview sincronizado.
- [ ] Undo remove o lote gerado; salvar/reabrir preserva as legendas.
- [ ] Exportação conserva Unicode, texto e sincronização.

Registrar revisão, sistema, hardware, modelo, duração da amostra, opção de idioma, tempo de execução e observações. Comparações de qualidade devem usar as mesmas condições.

## 9. Plano de desenvolvimento orientado à especificação

1. **Aprovação:** decidir rótulo, contrato restrito e ausência de persistência; alinhar mudança com mantenedores conforme `CONTRIBUTING.md`.
2. **Baseline:** testar o suporte automático atual com modelo multilíngue e áudio brasileiro; registrar resultados antes de alterar código.
3. **Testes primeiro:** adicionar testes de contrato e resolução; confirmar que falham pela ausência da funcionalidade, não pelo ambiente.
4. **Backend mínimo:** implementar campo, validação e parâmetros; fazer os testes passarem, preservando infraestrutura existente.
5. **UI:** ligar seletor ao request, adicionar orientação e atualizar internacionalização.
6. **Integração:** atualizar teste ignorado e executar a matriz com áudio real.
7. **Revisão:** verificar diff, checks e evidências; só então marcar requisitos como atendidos.

### Comandos para a futura implementação

Na pasta `src`, conforme os checks documentados do projeto:

```bash
cargo test -p concat-speech
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Na raiz, se houver strings novas:

```bash
python3 scripts/locales.py
```

No Windows, usar `python` se esse for o executável instalado. O script modifica inventários: revisar o diff antes de incluir os arquivos.

Baseline já disponível, em Bash, a partir de `src`, com arquivo real e modelo previamente instalado:

```bash
CONCAT_CAPTION_FILE="/caminho/amostra-pt-br.wav" CONCAT_CAPTION_MODEL=base cargo test -p concat-speech -- --ignored --nocapture captions_of_a_file
```

Após implementar a variável proposta, acrescentar `CONCAT_CAPTION_LANGUAGE=pt` para testar seleção explícita. Não confundir esse comando futuro com uma capacidade já disponível.

Compilação depende do ambiente Rust e das bibliotecas nativas do projeto, incluindo FFmpeg, CMake e C++. Falhas de ambiente devem ser registradas separadamente de falhas da funcionalidade.

## 10. Riscos e decisões pendentes

| Risco/decisão | Tratamento proposto |
| --- | --- |
| Rótulo regional sugere especialização inexistente | Explicar que o motor usa português `pt`; mantenedores podem preferir “Português”. |
| Autodetecção erra em trechos curtos | Oferecer seleção explícita; não prometer eliminar erros de reconhecimento. |
| Modelo inglês já selecionado | Bloquear combinação incompatível com mensagem acionável. |
| Modelo multilíngue menos preciso que o desejado | Avaliar `base` e `small` existentes; não incluir novos modelos neste escopo. |
| Áudio alterna idiomas | Manter comportamento atual; não prometer detecção por segmento. |
| Usuário deseja lembrar o idioma | Aprovar persistência como extensão separada, com default retrocompatível. |
| Uso externo de literais Rust do request | Documentar a mudança e atualizar callers; compatibilidade serde não resolve compilação fonte. |

### Aprovações solicitadas

- [ ] Reutilizar o `Transcriber` atual, sem criar uma implementação paralela pt-BR.
- [ ] Expor inicialmente Automático e Português (Brasil), usando `pt` no motor.
- [ ] Rejeitar modelos ingleses para português, sem troca ou download automático.
- [ ] Não persistir a seleção entre sessões no primeiro incremento.
- [ ] Aceitar apenas `auto`/`pt` no novo contrato; aliases regionais ficam fora do escopo.

## 11. Definição de concluído e limites desta análise

A futura implementação estará concluída somente quando os requisitos forem cobertos, os checks aplicáveis passarem e a matriz manual tiver evidências de execução, incluindo português real e regressão inglesa. Testes bloqueados devem ser reportados, não marcados como aprovados.

**Nesta entrega:** apenas leitura do guia e investigação estática do checkout para escrever a especificação. Nenhum áudio foi transcrito, modelo baixado, aplicativo executado ou ganho de precisão medido. Não foi feita busca de issues/PRs nem alinhamento com mantenedores. A viabilidade arquitetural é positiva; qualidade linguística e comportamento em execução continuam pendentes de validação.
