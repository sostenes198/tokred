[English](hooks.md) | **Português**

# Hooks

O `tokred configure` registra seis hooks no `settings.json`. Cada um executa `"<path>/tokred" hook <nome>` com timeout de 5 segundos (o `timeout` do Claude Code é em **segundos**).

| Hook | Evento do Claude Code | Matcher | Função | Fase |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | – | Classifica o prompt e sugere o subagente certo (uma linha, só quando a classe mudou) | F4 |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compara o subagente escolhido com a última classificação | F4 |
| `guard-read` | `PreToolUse` | `Read` | Bloqueia lockfiles, arquivos gerados, arquivos enormes sem `offset`/`limit` e releituras sem mudança | F3 |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Nega buscas amplas demais em repositórios grandes | F3 |
| `session-check` | `Stop` | – | Avisa o usuário (nunca o modelo) quando o contexto passa de `context_warn_tokens` | F5 |
| `session-start` | `SessionStart` | – | Registra a sessão; em `clear`/`compact` marca um corte para que releituras voltem a ser permitidas | F3 |

> **Fase F0:** os seis são instalados, mas são stubs fail-open que consomem o stdin e não imprimem nada, então configurar cedo nunca quebra nada.

A ferramenta de subagentes se chama `Agent` (era `Task` antes do Claude Code v2.1.63, e `Task` ainda funciona como alias), por isso o `enforce` casa os dois nomes.

## Formato das entradas

Cada hook do tokred fica em um **grupo próprio**, nunca misturado com os do rtk ou os seus:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }] }
```

O tokred reconhece as próprias entradas pelo comando: um executável chamado `tokred` (opcionalmente entre aspas, `tokred.exe` no Windows) seguido de `hook <nome>`. Só essas entradas são adicionadas, substituídas ou removidas.

## Modos

Todo hook respeita o modo definido em `[modes]` do `tokred.toml`: `off` (não faz nada), `warn` (permite e avisa) ou `enforce` (nega). Padrões: `guard_read = "enforce"`, o resto `"warn"`.

## Garantias

- **Fail-open:** em qualquer erro, panic ou timeout o hook sai com código 0 e sem saída, e registra o problema em `<dados>/logs/`. O tokred nunca bloqueia o Claude Code por bug próprio.
- **Rápido:** cada execução deve ficar abaixo de ~50 ms (p95). Nunca há rede dentro de um hook.
- **Contexto mínimo:** todo `additionalContext` tem uma ou duas linhas e só é emitido quando algo muda.
- **Nome de hook desconhecido sai com 0** (por exemplo após um downgrade), então o Claude Code nunca vê o código 2, que significa "bloquear".

## Coexistência com o rtk

O rtk atua no `PreToolUse` com o matcher `Bash`; o tokred nunca registra um matcher `Bash`. O `tokred doctor` detecta os hooks do rtk, confirma que não há sobreposição e avisa sobre qualquer outro hook que compartilhe um matcher com um hook do tokred.
