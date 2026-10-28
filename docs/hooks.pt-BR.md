[English](hooks.md) | **Português**

# Hooks

O `tokred configure` registra seis hooks no `settings.json`. Cada um executa `"<path>/tokred" hook <nome>` com timeout de 5 segundos (o `timeout` do Claude Code é em **segundos**). Os formatos de entrada e saída seguem a referência oficial, <https://code.claude.com/docs/en/hooks>.

| Hook | Evento do Claude Code | Matcher | Função | Fase |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | – | Classifica o prompt e sugere o subagente certo (uma linha, só quando a classe mudou) | F4 |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compara o subagente escolhido com a última classificação | F4 |
| `guard-read` | `PreToolUse` | `Read` | Bloqueia lockfiles, arquivos gerados, arquivos enormes sem `offset`/`limit` e releituras sem mudança | F3 |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Nega buscas amplas demais em repositórios grandes | F3 |
| `session-check` | `Stop` | – | Avisa o usuário (nunca o modelo) quando o contexto passa de `context_warn_tokens` | F5 |
| `session-start` | `SessionStart` | – | Registra a sessão; em `clear`/`compact` marca um corte para que releituras voltem a ser permitidas | F3 |

A ferramenta de subagentes se chama `Agent` (era `Task` antes do Claude Code v2.1.63, e `Task` ainda funciona como alias), por isso o `enforce` casa os dois nomes.

## Formato das entradas

Cada hook do tokred fica em um **grupo próprio**, nunca misturado com os do rtk ou os seus:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }] }
```

O tokred reconhece as próprias entradas pelo comando: um executável cujo nome de arquivo começa com `tokred` (opcionalmente entre aspas; `tokred.exe` no Windows; uma cópia com versão como `tokred-0.4.0` também vale) seguido de `hook <nome>`. Só essas entradas são adicionadas, substituídas ou removidas.

## Modos

Todo hook respeita o modo definido em `[modes]` do `tokred.toml`: `off` (não faz nada e não registra nada), `warn` (avisa, a chamada segue) ou `enforce` (nega). Padrões: `guard_read = "enforce"`, o resto `"warn"`. O `session-start` não tem modo: só registra a sessão.

Como cada resposta é enviada ao Claude Code:

| Resposta | Saída |
|---|---|
| Negar (`enforce`) | `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "<motivo curto>"}}` |
| Avisar (`warn`) | `{"hookSpecificOutput": {"hookEventName": "<evento>", "additionalContext": "<uma linha>"}}`, sem `permissionDecision` |
| Permitir | Sem saída (exit 0) |
| Mensagem ao usuário (`session-check`) | `{"systemMessage": "<texto>"}` |

O tokred nunca imprime `permissionDecision: "allow"`: o Claude Code documenta que ele **pula o prompt de permissão**, e o motivo só vai para o log de debug. "Permitir com um motivo curto" é portanto enviado como `additionalContext`.

## O que cada hook decide

### `guard-read`

1. **Lockfiles e arquivos gerados.** Os `deny_globs` (padrão: `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`, `*.min.js`, `*.map`, `dist/`, `build/`, `node_modules/`, `target/`, `.git/`) são comparados com o caminho **relativo ao `cwd` da sessão**, então um projeto que apenas mora sob uma pasta chamada `build` não é bloqueado. Globs inválidos são ignorados.
2. **Arquivo grande.** Acima de `max_read_bytes` sem `offset` nem `limit`: negado, pedindo uma leitura por faixa. Imagens e PDFs (`png`, `jpg`, `jpeg`, `gif`, `webp`, `bmp`, `pdf`) ficam isentos: o `Read` os exibe nativamente.
3. **Releitura.** O blake3 do arquivo é comparado com leituras anteriores da mesma sessão para o mesmo `path`, `offset` e `limit`; uma idêntica é negada (o conteúdo já está no contexto). Arquivos acima de 16 MiB não são hasheados (a checagem é pulada), nem leituras com qualquer argumento além de `file_path`, `offset` e `limit` (como `pages` de um PDF), porque devolvem outra coisa para a mesma chave. Um `clear` ou `compact` (`session-start`) é um corte: só contam leituras **estritamente depois** do corte, então uma leitura no mesmo segundo é permitida (o lado fail-open).
4. A leitura e os bytes evitados são registrados (`bytes_saved`; no modo `warn` nada é evitado, então 0). O tamanho de uma leitura por faixa é estimado em 80 bytes por linha pedida.

Hooks disparados dentro de um **subagente** trazem um `agent_id`; as leituras deles são rastreadas por `sessão/agente`, porque o contexto do subagente não contém o que o pai leu.

### `guard-search`

Um `Grep` sem `path`, `glob` nem `type`, ou um `Glob` cujo padrão casa tudo (`*`, `**`, `**/*`) e sem `path`, é "amplo". Só buscas amplas contam arquivos: a raiz do repositório é o ancestral mais próximo com `.git` (senão, o diretório da busca), os arquivos abaixo dela são contados (ignorando `.git`, `node_modules`, `target`, `dist`, `build`, `.venv`, `__pycache__`, `.next`) até `large_repo_files + 1`, e a contagem fica no [cache em camadas](cache.pt-BR.md) com chave por raiz, limite e data de modificação da raiz (uma mudança no topo do repositório, portanto, dispara uma nova contagem). Uma contagem que leve mais de 100 ms é abandonada e a busca é permitida. Acima de `large_repo_files` a busca é negada (`enforce`) ou desaconselhada (`warn`, o padrão).

### `route`

Um classificador heurístico (sem LLM) sobre palavras-chave em pt-BR e inglês (arquitetura/design, plano/planejar, implementar/corrigir/bug/refatorar, revisar, onde fica/encontrar/buscar, resumir), o tamanho do prompt e menções a arquivos. Prompts com menos de 30 caracteres que não mencionam arquivo são tarefas pontuais e nunca recebem sugestão. Um `additionalContext` de uma linha só é emitido quando a classe mudou desde o último prompt acionável da sessão, por exemplo:

```
tokred: implementation task -> delegate to the tokred-dev subagent if it involves more than one edit.
```

A classe de um prompt é guardada em cache pelo blake3 do prompt, então repetir um prompt é um acerto de cache. Toda execução registra um evento `route` cujo `decision` é a classe (`architecture`, `planning`, `implementation`, `review`, `search`, `summary`, `other`). A tabela `classifications` só guarda classes acionáveis, então um "ok, pode seguir" entre dois prompts de implementação não faz a sugestão se repetir. O `route` não pode negar: o modo `enforce` se comporta como `warn`.

### `enforce`

Para a ferramenta de subagentes, compara o `subagent_type` escolhido com a última classificação da sessão. Agentes que não são do tokred são sempre permitidos, assim como uma sessão sem nada classificado. Arquitetura e planejamento aceitam qualquer um dos dois agentes opus. Em caso de divergência, `warn` adiciona um aviso curto e `enforce` nega, nomeando o agente certo. Só a conversa principal é verificada (hooks com `agent_id` são ignorados).

### `session-start`

Registra a sessão e o diretório de trabalho. Em `source` = `clear` ou `compact` marca o corte usado pelo `guard-read`. `startup`, `resume` e `fork` mantêm o contexto anterior, então não são cortes. Não imprime nada: no `SessionStart`, qualquer coisa no stdout seria adicionada ao contexto do modelo.

### `session-check`

Pega o `usage` da última mensagem do assistente da conversa principal no transcript (`transcript_path`; só o fim do arquivo é lido) e estima o contexto como `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`. Acima de `context_warn_tokens` imprime uma `systemMessage` para o **usuário** sugerindo `/compact` ou `/clear`. Nunca bloqueia o stop, nunca usa `decision` nem `additionalContext` no `Stop` (isso faria o Claude continuar) e avisa uma vez por sessão: de novo só depois que o contexto crescer metade do limite, ou após um `clear`/`compact`. O estado do aviso fica no cache em camadas (entrada L1 de 4 h). O Claude Code grava o transcript de forma assíncrona, então o número pode atrasar uma mensagem.

## Garantias

- **Fail-open:** em qualquer erro, panic, entrada inválida ou timeout o hook sai com código 0 e sem saída, e registra o problema em `<dados>/logs/`. Um guarda de tempo (2 s, `TOKRED_HOOK_GUARD_MS` o altera nos testes) registra `timeout_guard` e sai com 0 se um hook travar. Se a pasta de dados ou o `tokred.db` não existir, o hook não faz nada e nunca os cria. Um `tokred.toml` inválido significa os padrões.
- **Rápido:** cada execução fica bem abaixo de ~50 ms (p95): veja `tokred doctor --bench`. Nunca há rede dentro de um hook.
- **Medido:** toda execução é registrada em `hook_events` com `duration_us` (medido desde a primeira linha do `main`) e `outcome` (`ok`, `error`, `panic`, `timeout_guard`). A escrita é o **último** passo, depois que a resposta foi escrita e descarregada. O processo não consegue fechar o próprio stdout (`unsafe` é proibido), então o Claude Code só vê o fim da execução quando o processo termina; a escrita em si leva uma fração de milissegundo.
- **Contexto mínimo:** todo `additionalContext` tem uma linha e só é emitido quando algo muda.
- **Nome de hook desconhecido sai com 0** (por exemplo após um downgrade), então o Claude Code nunca vê o código 2, que significa "bloquear".

## Coexistência com o rtk

O rtk atua no `PreToolUse` com o matcher `Bash`; o tokred nunca registra um matcher `Bash`. O `tokred doctor` detecta os hooks do rtk, confirma que não há sobreposição e avisa sobre qualquer outro hook que compartilhe um matcher com um hook do tokred.
